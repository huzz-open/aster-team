package mariadb

import (
	"context"
	"database/sql"
	"fmt"
	"net"
	"time"

	"aster.local/team/operations/backend/internal/config"
	mysqldriver "github.com/go-sql-driver/mysql"
)

func EnsureDatabase(ctx context.Context, cfg config.Database) error {
	driverConfig := mysqlConfig(cfg)
	driverConfig.DBName = ""
	connector, err := mysqldriver.NewConnector(driverConfig)
	if err != nil {
		return fmt.Errorf("configure MariaDB server connection: %w", err)
	}
	db := sql.OpenDB(connector)
	defer db.Close()
	if err := db.PingContext(ctx); err != nil {
		return fmt.Errorf("connect to MariaDB server: %w", err)
	}
	statement := "CREATE DATABASE IF NOT EXISTS `" + cfg.Name + "` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"
	if _, err := db.ExecContext(ctx, statement); err != nil {
		return fmt.Errorf("create operations database: %w", err)
	}
	return nil
}

func Open(ctx context.Context, cfg config.Database) (*sql.DB, error) {
	connector, err := mysqldriver.NewConnector(mysqlConfig(cfg))
	if err != nil {
		return nil, fmt.Errorf("configure MariaDB connection: %w", err)
	}
	db := sql.OpenDB(connector)
	db.SetMaxOpenConns(cfg.MaxOpenConns)
	db.SetMaxIdleConns(cfg.MaxIdleConns)
	db.SetConnMaxLifetime(cfg.ConnMaxLifetime)
	db.SetConnMaxIdleTime(cfg.ConnMaxIdleTime)
	if err := db.PingContext(ctx); err != nil {
		db.Close()
		return nil, fmt.Errorf("connect to operations database: %w", err)
	}
	return db, nil
}

func mysqlConfig(cfg config.Database) *mysqldriver.Config {
	tlsConfig := "false"
	if cfg.TLS {
		tlsConfig = "true"
	}
	return &mysqldriver.Config{
		User:                 cfg.User,
		Passwd:               cfg.Password,
		Net:                  "tcp",
		Addr:                 net.JoinHostPort(cfg.Host, fmt.Sprintf("%d", cfg.Port)),
		DBName:               cfg.Name,
		Collation:            "utf8mb4_unicode_ci",
		Loc:                  cfg.Location,
		ParseTime:            true,
		Timeout:              cfg.ConnectTimeout,
		ReadTimeout:          cfg.ReadTimeout,
		WriteTimeout:         cfg.WriteTimeout,
		TLSConfig:            tlsConfig,
		AllowNativePasswords: true,
		CheckConnLiveness:    true,
		Params: map[string]string{
			"time_zone": "'+00:00'",
		},
	}
}

func Health(ctx context.Context, db *sql.DB) error {
	ctx, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()
	return db.PingContext(ctx)
}
