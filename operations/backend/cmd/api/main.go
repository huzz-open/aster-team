package main

import (
	"context"
	"errors"
	"flag"
	"log/slog"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	operationsfs "aster.local/team/operations/backend/internal/adapters/filesystem"
	"aster.local/team/operations/backend/internal/adapters/githubactions"
	"aster.local/team/operations/backend/internal/adapters/httpcatalog"
	"aster.local/team/operations/backend/internal/adapters/licensing"
	"aster.local/team/operations/backend/internal/adapters/mariadb"
	"aster.local/team/operations/backend/internal/adapters/releaseverification"
	"aster.local/team/operations/backend/internal/adapters/signing"
	"aster.local/team/operations/backend/internal/adapters/upgradetarget"
	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/config"
	"aster.local/team/operations/backend/internal/transport/httpapi"
)

func main() {
	envFile := flag.String("env-file", ".env", "environment file used for local development")
	createDatabase := flag.Bool("create-database", false, "create the configured Operations database if it does not exist")
	migrateOnly := flag.Bool("migrate-only", false, "apply schema migrations and exit")
	grantCommercialAdmin := flag.String("grant-commercial-admin", "", "grant commercial management to an explicit existing operator during local migration")
	confirmDatabase := flag.String("confirm-database", "", "exact database name required for local permission grants")
	flag.Parse()

	logger := slog.New(slog.NewJSONHandler(os.Stdout, nil))
	cfg, err := config.Load(*envFile)
	if err != nil {
		logger.Error("invalid Operations API configuration", "error", err)
		os.Exit(1)
	}
	if *grantCommercialAdmin != "" && (!*migrateOnly || *confirmDatabase != cfg.Database.Name) {
		logger.Error("commercial permission grants require --migrate-only and --confirm-database matching the configured database")
		os.Exit(1)
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	if *createDatabase || cfg.CreateDatabase {
		if err := mariadb.EnsureDatabase(ctx, cfg.Database); err != nil {
			logger.Error("create Operations database failed", "host", cfg.Database.Host, "port", cfg.Database.Port, "database", cfg.Database.Name, "error", err)
			os.Exit(1)
		}
	}
	database, err := mariadb.Open(ctx, cfg.Database)
	if err != nil {
		logger.Error("open Operations database failed", "host", cfg.Database.Host, "port", cfg.Database.Port, "database", cfg.Database.Name, "error", err)
		os.Exit(1)
	}
	defer database.Close()
	if *migrateOnly || cfg.AutoMigrate {
		if err := mariadb.Migrate(ctx, database); err != nil {
			logger.Error("migrate Operations database failed", "database", cfg.Database.Name, "error", err)
			os.Exit(1)
		}
	}
	if *migrateOnly {
		if *grantCommercialAdmin != "" {
			if err := mariadb.NewStore(database).GrantCommercialAdministrator(ctx, *grantCommercialAdmin, time.Now().UTC()); err != nil {
				logger.Error("grant commercial administrator failed", "operator_id", *grantCommercialAdmin, "error", err)
				os.Exit(1)
			}
			logger.Info("commercial administrator permissions granted", "operator_id", *grantCommercialAdmin, "database", cfg.Database.Name)
		}
		logger.Info("Operations database migrations applied", "database_engine", cfg.Database.Driver, "database", cfg.Database.Name)
		return
	}

	store := mariadb.NewStore(database)
	serviceOptions := []application.Option{}
	artifactStore, err := operationsfs.NewArtifactStore(cfg.ArtifactRoot)
	if err != nil {
		logger.Error("configure Operations artifact store failed", "error", err)
		os.Exit(1)
	}
	serviceOptions = append(serviceOptions, application.WithArtifactStore(artifactStore))
	publicationVerifiers := map[string]application.PublicationVerifier{}
	for environment, origin := range map[string]string{"local": cfg.PublicationLocalOrigin, "production": cfg.PublicationProductionOrigin} {
		if origin == "" {
			continue
		}
		verifier, err := httpcatalog.New(environment, origin)
		if err != nil {
			logger.Error("configure publication verifier failed", "environment", environment, "error", err)
			os.Exit(1)
		}
		publicationVerifiers[environment] = verifier
	}
	serviceOptions = append(serviceOptions, application.WithPublicationVerifiers(publicationVerifiers))
	serviceOptions = append(serviceOptions, application.WithQuotationEnvironment(cfg.QuotationEnvironment), application.WithFulfillmentEnvironment(cfg.FulfillmentEnvironment))
	if cfg.PublicCatalogRoot != "" {
		catalogExporter, err := operationsfs.NewPublicCatalogExporter(cfg.PublicCatalogRoot)
		if err != nil {
			logger.Error("configure public catalog export failed", "error", err)
			os.Exit(1)
		}
		defer catalogExporter.Close()
		serviceOptions = append(serviceOptions, application.WithPublicCatalogExporter(catalogExporter))
	}
	v2Signers, err := licensing.LoadV2Signers(cfg.LicenseV2SignersJSON, nil)
	if err != nil {
		logger.Error("configure v2 license signers failed", "error", err)
		os.Exit(1)
	}
	v2Verifiers, err := licensing.LoadV2Verifiers(cfg.LicenseV2VerifiersJSON, v2Signers, nil)
	if err != nil {
		logger.Error("configure v2 license verification failed", "error", err)
		os.Exit(1)
	}
	serviceOptions = append(serviceOptions, application.WithV2LicenseSigners(v2Signers), application.WithV2LicenseVerifiers(v2Verifiers))
	if cfg.CustomerReferenceSecret != "" {
		customerRefs, err := signing.NewHMACCustomerReferenceSource(cfg.CustomerReferenceSecret)
		if err != nil {
			logger.Error("configure customer reference source failed", "error", err)
			os.Exit(1)
		}
		serviceOptions = append(serviceOptions, application.WithCustomerReferenceSource(customerRefs))
	}
	if cfg.GitHubApp.Enabled {
		releaseOrchestrator, err := githubactions.New(cfg.GitHubApp)
		if err != nil {
			logger.Error("configure GitHub release orchestrator failed", "error", err)
			os.Exit(1)
		}
		serviceOptions = append(serviceOptions, application.WithReleaseOrchestrator(releaseOrchestrator))
		releaseVerifier, err := releaseverification.New(releaseOrchestrator, artifactStore, cfg.ReleaseVerification)
		if err != nil {
			logger.Error("configure independent release artifact verifier failed", "error", err)
			os.Exit(1)
		}
		serviceOptions = append(serviceOptions, application.WithReleaseArtifactVerifier(releaseVerifier))
	}
	if cfg.GitHubPublisher.Enabled {
		releasePublisher, err := githubactions.NewPublisher(cfg.GitHubPublisher, artifactStore, cfg.ReleaseVerification.MaxArtifactBytes)
		if err != nil {
			logger.Error("configure dedicated GitHub release publisher failed", "error", err)
			os.Exit(1)
		}
		serviceOptions = append(serviceOptions, application.WithReleasePublisher(releasePublisher))
	}
	if cfg.UpgradeCredentialKey != "" {
		secrets, err := upgradetarget.NewSecrets(cfg.UpgradeCredentialKey)
		if err != nil {
			logger.Error("configure upgrade credential encryption failed")
			os.Exit(1)
		}
		serviceOptions = append(serviceOptions, application.WithEnvironmentUpgrades(store, secrets, upgradetarget.Factory{}))
	}
	service := application.NewService(store, cfg.SessionTTL, serviceOptions...)
	for range 2 {
		go service.RunEnvironmentUpgrades(ctx)
	}
	if err := service.BootstrapAdmin(ctx, cfg.BootstrapEmail, cfg.BootstrapPassword); err != nil {
		logger.Error("bootstrap Operations administrator failed", "error", err)
		os.Exit(1)
	}
	if cfg.GitHubApp.Enabled || cfg.GitHubPublisher.Enabled {
		go monitorReleaseTasks(ctx, service, cfg.GitHubApp.PollInterval, logger)
	}
	handler := httpapi.New(service, func(ctx context.Context) error { return mariadb.Health(ctx, database) }, logger, httpapi.Config{
		CookieSecure: cfg.CookieSecure, TrustedOrigins: cfg.TrustedOrigins, BodyMaxSize: cfg.RequestBodyMaxSize,
	})
	server := &http.Server{
		Addr:              cfg.Address,
		Handler:           handler,
		ReadHeaderTimeout: 10 * time.Second,
		ReadTimeout:       30 * time.Second,
		WriteTimeout:      30 * time.Second,
		IdleTimeout:       90 * time.Second,
		MaxHeaderBytes:    1 << 20,
	}
	go func() {
		<-ctx.Done()
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		if err := server.Shutdown(shutdownCtx); err != nil {
			logger.Error("Operations API shutdown failed", "error", err)
		}
	}()
	logger.Info("Aster Operations API listening", "address", cfg.Address, "database_engine", cfg.Database.Driver, "database_host", cfg.Database.Host, "database_port", cfg.Database.Port, "database", cfg.Database.Name)
	if err := server.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
		logger.Error("Operations API stopped", "error", err)
		os.Exit(1)
	}
}

func monitorReleaseTasks(ctx context.Context, service *application.Service, interval time.Duration, logger *slog.Logger) {
	ticker := time.NewTicker(interval)
	defer ticker.Stop()
	for {
		syncCtx, cancel := context.WithTimeout(ctx, 2*time.Minute)
		for _, err := range service.SyncPendingReleaseTasks(syncCtx, 25) {
			logger.Warn("release task synchronization failed", "error", err)
		}
		for _, err := range service.ResumePendingReleasePublishes(syncCtx, 10) {
			logger.Warn("release publishing recovery failed", "error", err)
		}
		cancel()
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
		}
	}
}
