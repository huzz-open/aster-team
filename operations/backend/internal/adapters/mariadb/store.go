package mariadb

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
	"time"

	"aster.local/team/operations/backend/internal/application"
	"aster.local/team/operations/backend/internal/domain"
	"aster.local/team/operations/backend/internal/ports"
	mysqldriver "github.com/go-sql-driver/mysql"
)

type Store struct {
	db *sql.DB
}

type queryContext interface {
	QueryContext(context.Context, string, ...any) (*sql.Rows, error)
}

func NewStore(db *sql.DB) *Store {
	return &Store{db: db}
}

func (store *Store) ListLicenseRecords(ctx context.Context, limit int) ([]ports.LicenseRecord, error) {
	rows, err := store.db.QueryContext(ctx, `SELECT policy_json,status,transfer_limit,transfer_count,created_at,updated_at
		FROM license_records ORDER BY created_at DESC LIMIT ?`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := make([]ports.LicenseRecord, 0)
	for rows.Next() {
		var policyJSON []byte
		var item ports.LicenseRecord
		if err = rows.Scan(&policyJSON, &item.Status, &item.TransferLimit, &item.TransferCount, &item.CreatedAt, &item.UpdatedAt); err != nil {
			return nil, err
		}
		if err = json.Unmarshal(policyJSON, &item.LicensePolicy); err != nil {
			return nil, err
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func (store *Store) LicenseForIssuance(ctx context.Context, licenseID string) (ports.LicenseRecord, error) {
	var policyJSON []byte
	var item ports.LicenseRecord
	err := store.db.QueryRowContext(ctx, `SELECT policy_json,status,transfer_limit,transfer_count,created_at,updated_at
		FROM license_records WHERE license_id=?`, licenseID).
		Scan(&policyJSON, &item.Status, &item.TransferLimit, &item.TransferCount, &item.CreatedAt, &item.UpdatedAt)
	if err != nil {
		return ports.LicenseRecord{}, err
	}
	if err = json.Unmarshal(policyJSON, &item.LicensePolicy); err != nil {
		return ports.LicenseRecord{}, err
	}
	return item, nil
}

func (store *Store) SaveLicenseIssuance(ctx context.Context, issuance ports.LicenseIssuance, operatorID string) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	var status string
	var transferLimit int
	var policyJSON []byte
	if err = tx.QueryRowContext(ctx, `SELECT policy_json,status,transfer_limit FROM license_records WHERE license_id=? FOR UPDATE`, issuance.LicenseID).Scan(&policyJSON, &status, &transferLimit); err != nil {
		return err
	}
	if status != "active" {
		return errors.New("license is not active")
	}
	var policy ports.LicensePolicy
	if err = json.Unmarshal(policyJSON, &policy); err != nil {
		return err
	}
	if !policy.ValidUntil.After(issuance.IssuedAt) || !policy.ValidUntil.Equal(issuance.ExpiresAt) {
		return errors.New("license validity period changed before issuance was saved")
	}
	if policy.SourceType == "trial" {
		var successor string
		if err := tx.QueryRowContext(ctx, "SELECT fulfillment_id FROM commercial_paid_lifecycle_sources WHERE source_namespace='trial' AND source_id=? FOR UPDATE", policy.SourceID).Scan(&successor); err == nil {
			return errors.New("trial installation is frozen by an approved paid conversion")
		} else if !errors.Is(err, sql.ErrNoRows) {
			return err
		}
	}
	var lastInstallationID, lastFingerprint string
	var lastSequence int
	lastErr := tx.QueryRowContext(ctx, `SELECT installation_id,machine_fingerprint_sha256,transfer_sequence FROM license_issuances
		WHERE license_id=? ORDER BY issued_at DESC,id DESC LIMIT 1 FOR UPDATE`, issuance.LicenseID).
		Scan(&lastInstallationID, &lastFingerprint, &lastSequence)
	expectedSequence := 0
	if lastErr == nil {
		expectedSequence = lastSequence
		if lastInstallationID != issuance.InstallationID || lastFingerprint != issuance.MachineFingerprintSHA256 {
			expectedSequence++
		}
	} else if !errors.Is(lastErr, sql.ErrNoRows) {
		return lastErr
	}
	if issuance.TransferSequence != expectedSequence || issuance.TransferSequence > transferLimit {
		return errors.New("license transfer limit or sequence is invalid")
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO license_issuances
		(id,license_id,request_id,installation_id,machine_fingerprint_sha256,transfer_sequence,document_json,sha256,issued_by,issued_at,expires_at)
		VALUES(?,?,?,?,?,?,?,?,?,?,?)`, issuance.ID, issuance.LicenseID, issuance.RequestID, issuance.InstallationID,
		issuance.MachineFingerprintSHA256, issuance.TransferSequence, issuance.Document, issuance.SHA256, operatorID, issuance.IssuedAt, issuance.ExpiresAt); err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, `UPDATE license_records SET transfer_count=GREATEST(transfer_count,?),updated_at=? WHERE license_id=?`, issuance.TransferSequence, issuance.IssuedAt, issuance.LicenseID); err != nil {
		return err
	}
	payload, _ := json.Marshal(map[string]any{"issuance_id": issuance.ID, "request_id": issuance.RequestID, "transfer_sequence": issuance.TransferSequence, "sha256": issuance.SHA256})
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events(id,operator_id,action,resource_type,resource_id,payload_json,created_at)
		VALUES(?,?, 'license.issued','license',?,?,?)`, operationAuditID(issuance.ID, "issued"), operatorID, issuance.LicenseID, payload, issuance.IssuedAt); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) ListLicenseIssuances(ctx context.Context, licenseID string) ([]ports.LicenseIssuance, error) {
	rows, err := store.db.QueryContext(ctx, `SELECT id,license_id,request_id,installation_id,machine_fingerprint_sha256,transfer_sequence,
		document_json,sha256,issued_at,expires_at FROM license_issuances WHERE license_id=? ORDER BY issued_at DESC,id DESC`, licenseID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := make([]ports.LicenseIssuance, 0)
	for rows.Next() {
		var item ports.LicenseIssuance
		if err = rows.Scan(&item.ID, &item.LicenseID, &item.RequestID, &item.InstallationID, &item.MachineFingerprintSHA256,
			&item.TransferSequence, &item.Document, &item.SHA256, &item.IssuedAt, &item.ExpiresAt); err != nil {
			return nil, err
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func (store *Store) OperatorCount(ctx context.Context) (int64, error) {
	var count int64
	err := store.db.QueryRowContext(ctx, "SELECT COUNT(*) FROM operators").Scan(&count)
	return count, err
}

func (store *Store) CreateOperator(ctx context.Context, params application.CreateOperatorParams) (domain.Operator, error) {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return domain.Operator{}, err
	}
	defer tx.Rollback()
	_, err = tx.ExecContext(ctx, `INSERT INTO operators
        (id, email, normalized_email, display_name, password_hash, status, password_change_required, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, 'active', ?, ?, ?)`,
		params.ID, params.Email, params.NormalizedEmail, params.DisplayName, params.PasswordHash, params.PasswordChangeRequired, params.CreatedAt, params.CreatedAt)
	if err != nil {
		return domain.Operator{}, err
	}
	for _, permission := range append(application.BootstrapReleasePermissions(), application.BootstrapCommercialPermissions()...) {
		if _, err = tx.ExecContext(ctx, `INSERT INTO operator_permissions (operator_id, permission_code, granted_at) VALUES (?, ?, ?)`,
			params.ID, permission, params.CreatedAt); err != nil {
			return domain.Operator{}, err
		}
	}
	if err = tx.Commit(); err != nil {
		return domain.Operator{}, err
	}
	return domain.Operator{ID: params.ID, Email: params.Email, DisplayName: params.DisplayName, Status: "active", PasswordChangeRequired: params.PasswordChangeRequired, CreatedAt: params.CreatedAt}, nil
}

func (store *Store) HasPermission(ctx context.Context, operatorID, permission string) (bool, error) {
	var matched int
	err := store.db.QueryRowContext(ctx, `SELECT COUNT(*) FROM operator_permissions WHERE operator_id = ? AND permission_code = ?`,
		operatorID, permission).Scan(&matched)
	return matched == 1, err
}

func (store *Store) FindOperatorForLogin(ctx context.Context, normalizedEmail string) (domain.Operator, string, error) {
	var operator domain.Operator
	var passwordHash string
	err := store.db.QueryRowContext(ctx, `SELECT id, email, display_name, status, password_change_required, created_at, password_hash
        FROM operators WHERE normalized_email = ?`, normalizedEmail).Scan(
		&operator.ID, &operator.Email, &operator.DisplayName, &operator.Status, &operator.PasswordChangeRequired, &operator.CreatedAt, &passwordHash)
	return operator, passwordHash, err
}

func (store *Store) CreateSession(ctx context.Context, params application.CreateSessionParams) error {
	_, err := store.db.ExecContext(ctx, `INSERT INTO operator_sessions
        (id, operator_id, token_hash, csrf_hash, expires_at, created_at, last_seen_at)
        VALUES (?, ?, ?, ?, ?, ?, ?)`,
		params.ID, params.OperatorID, params.TokenHash[:], params.CSRFHash[:], params.ExpiresAt, params.CreatedAt, params.CreatedAt)
	return err
}

func (store *Store) AuthenticateSession(ctx context.Context, tokenHash [32]byte, now time.Time) (domain.AuthenticatedOperator, error) {
	var operator domain.AuthenticatedOperator
	var csrfHash []byte
	err := store.db.QueryRowContext(ctx, `SELECT o.id, o.email, o.display_name, o.status, o.password_change_required, o.created_at,
        s.id, s.csrf_hash, s.expires_at
        FROM operator_sessions s JOIN operators o ON o.id = s.operator_id
        WHERE s.token_hash = ? AND s.expires_at > ?`, tokenHash[:], now).Scan(
		&operator.ID, &operator.Email, &operator.DisplayName, &operator.Status, &operator.PasswordChangeRequired, &operator.CreatedAt,
		&operator.SessionID, &csrfHash, &operator.ExpiresAt)
	if err != nil {
		return domain.AuthenticatedOperator{}, err
	}
	if len(csrfHash) != len(operator.CSRFHash) {
		return domain.AuthenticatedOperator{}, errors.New("stored CSRF hash has invalid length")
	}
	copy(operator.CSRFHash[:], csrfHash)
	return operator, nil
}

func (store *Store) DeleteSession(ctx context.Context, tokenHash [32]byte) error {
	_, err := store.db.ExecContext(ctx, "DELETE FROM operator_sessions WHERE token_hash = ?", tokenHash[:])
	return err
}

func (store *Store) DeleteExpiredSessions(ctx context.Context, now time.Time) error {
	_, err := store.db.ExecContext(ctx, "DELETE FROM operator_sessions WHERE expires_at <= ?", now)
	return err
}

func (store *Store) UpdateOperatorPassword(ctx context.Context, operatorID, currentSessionID, passwordHash string, now time.Time) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	result, err := tx.ExecContext(ctx, `UPDATE operators SET password_hash = ?, password_change_required = FALSE, updated_at = ? WHERE id = ? AND status = 'active'`, passwordHash, now, operatorID)
	if err != nil {
		return err
	}
	if changed, _ := result.RowsAffected(); changed != 1 {
		return sql.ErrNoRows
	}
	if _, err = tx.ExecContext(ctx, "DELETE FROM operator_sessions WHERE operator_id = ? AND id <> ?", operatorID, currentSessionID); err != nil {
		return err
	}
	payload, _ := json.Marshal(map[string]any{"other_sessions_revoked": true})
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events
		(id, operator_id, action, resource_type, resource_id, payload_json, created_at)
		VALUES (?, ?, 'operator.password_changed', 'operator', ?, ?, ?)`, currentSessionID+"_password_changed", operatorID, operatorID, payload, now); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) CreateCustomer(ctx context.Context, customer domain.Customer, operatorID string) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	_, err = tx.ExecContext(ctx, `INSERT INTO customers
        (id, name, legal_name, status, contact_name, contact_email, contact_phone, contact_wechat, notes, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
		customer.ID, customer.Name, customer.LegalName, customer.Status, customer.ContactName, customer.ContactEmail,
		customer.ContactPhone, customer.ContactWeChat, customer.Notes, customer.CreatedAt, customer.UpdatedAt)
	if err != nil {
		return err
	}
	payload, err := json.Marshal(customer)
	if err != nil {
		return err
	}
	_, err = tx.ExecContext(ctx, `INSERT INTO audit_events
        (id, operator_id, action, resource_type, resource_id, payload_json, created_at)
        VALUES (?, ?, 'customer.created', 'customer', ?, ?, ?)`,
		customer.ID+"_created", operatorID, customer.ID, payload, customer.CreatedAt)
	if err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) ListCustomers(ctx context.Context, after string, limit int) ([]domain.Customer, error) {
	return listCustomers(ctx, store.db, after, limit)
}

func listCustomers(ctx context.Context, database queryContext, after string, limit int) ([]domain.Customer, error) {
	query := `SELECT id, name, legal_name, status, contact_name, contact_email, contact_phone, contact_wechat, notes, created_at, updated_at
        FROM customers`
	args := []any{}
	if after != "" {
		query += " WHERE id < ?"
		args = append(args, after)
	}
	query += " ORDER BY id DESC LIMIT ?"
	args = append(args, limit)
	rows, err := database.QueryContext(ctx, query, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	customers := make([]domain.Customer, 0, limit)
	for rows.Next() {
		var customer domain.Customer
		if err := rows.Scan(&customer.ID, &customer.Name, &customer.LegalName, &customer.Status, &customer.ContactName,
			&customer.ContactEmail, &customer.ContactPhone, &customer.ContactWeChat, &customer.Notes, &customer.CreatedAt, &customer.UpdatedAt); err != nil {
			return nil, err
		}
		customers = append(customers, customer)
	}
	return customers, rows.Err()
}

func (store *Store) UpdateCustomer(ctx context.Context, customer domain.Customer, operatorID string) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	var before domain.Customer
	if err = tx.QueryRowContext(ctx, `SELECT id, name, legal_name, status, contact_name, contact_email, contact_phone, contact_wechat, notes, created_at, updated_at FROM customers WHERE id = ? FOR UPDATE`, customer.ID).Scan(&before.ID, &before.Name, &before.LegalName, &before.Status, &before.ContactName, &before.ContactEmail, &before.ContactPhone, &before.ContactWeChat, &before.Notes, &before.CreatedAt, &before.UpdatedAt); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return application.ErrNotFound
		}
		return err
	}
	if _, err = tx.ExecContext(ctx, `UPDATE customers SET name=?, legal_name=?, status=?, contact_name=?, contact_email=?, contact_phone=?, contact_wechat=?, notes=?, updated_at=? WHERE id=?`, customer.Name, customer.LegalName, customer.Status, customer.ContactName, customer.ContactEmail, customer.ContactPhone, customer.ContactWeChat, customer.Notes, customer.UpdatedAt, customer.ID); err != nil {
		return err
	}
	payload, _ := json.Marshal(map[string]any{"before": before, "after": customer})
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events (id, operator_id, action, resource_type, resource_id, payload_json, created_at) VALUES (?, ?, 'customer.updated', 'customer', ?, ?, ?)`, customer.ID+"_updated_"+customer.UpdatedAt.Format("150405.000000"), operatorID, customer.ID, payload, customer.UpdatedAt); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) GetCustomerProfile(ctx context.Context, customerID string) (domain.CustomerProfile, error) {
	var profile domain.CustomerProfile
	err := store.db.QueryRowContext(ctx, `SELECT id, name, legal_name, status, contact_name, contact_email, contact_phone, contact_wechat, notes, created_at, updated_at FROM customers WHERE id = ?`, customerID).Scan(&profile.Customer.ID, &profile.Customer.Name, &profile.Customer.LegalName, &profile.Customer.Status, &profile.Customer.ContactName, &profile.Customer.ContactEmail, &profile.Customer.ContactPhone, &profile.Customer.ContactWeChat, &profile.Customer.Notes, &profile.Customer.CreatedAt, &profile.Customer.UpdatedAt)
	if errors.Is(err, sql.ErrNoRows) {
		return profile, application.ErrNotFound
	}
	if err != nil {
		return profile, err
	}
	rows, err := store.db.QueryContext(ctx, `SELECT id, customer_id, name, email, phone, wechat, role_title, is_primary, created_at, updated_at FROM contacts WHERE customer_id = ? ORDER BY is_primary DESC, created_at`, customerID)
	if err != nil {
		return profile, err
	}
	defer rows.Close()
	profile.Contacts = []domain.Contact{}
	for rows.Next() {
		var item domain.Contact
		if err := rows.Scan(&item.ID, &item.CustomerID, &item.Name, &item.Email, &item.Phone, &item.WeChat, &item.RoleTitle, &item.IsPrimary, &item.CreatedAt, &item.UpdatedAt); err != nil {
			return profile, err
		}
		profile.Contacts = append(profile.Contacts, item)
	}
	if err := rows.Err(); err != nil {
		return profile, err
	}
	var billing domain.BillingProfile
	err = store.db.QueryRowContext(ctx, `SELECT id, customer_id, invoice_title, tax_identifier, billing_email, address, created_at, updated_at FROM billing_profiles WHERE customer_id = ?`, customerID).Scan(&billing.ID, &billing.CustomerID, &billing.InvoiceTitle, &billing.TaxIdentifier, &billing.BillingEmail, &billing.Address, &billing.CreatedAt, &billing.UpdatedAt)
	if err == nil {
		profile.Billing = &billing
	} else if !errors.Is(err, sql.ErrNoRows) {
		return profile, err
	}
	return profile, nil
}

func (store *Store) CreateContact(ctx context.Context, contact domain.Contact, operatorID string) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	var lockedCustomerID string
	if err = tx.QueryRowContext(ctx, `SELECT id FROM customers WHERE id=? FOR UPDATE`, contact.CustomerID).Scan(&lockedCustomerID); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return application.ErrNotFound
		}
		return err
	}
	if contact.IsPrimary {
		if _, err = tx.ExecContext(ctx, `UPDATE contacts SET is_primary=FALSE, updated_at=? WHERE customer_id=? AND is_primary=TRUE`, contact.CreatedAt, contact.CustomerID); err != nil {
			return err
		}
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO contacts (id, customer_id, name, email, phone, wechat, role_title, is_primary, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`, contact.ID, contact.CustomerID, contact.Name, contact.Email, contact.Phone, contact.WeChat, contact.RoleTitle, contact.IsPrimary, contact.CreatedAt, contact.UpdatedAt); err != nil {
		return err
	}
	payload, _ := json.Marshal(contact)
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events (id, operator_id, action, resource_type, resource_id, payload_json, created_at) VALUES (?, ?, 'contact.created', 'contact', ?, ?, ?)`, contact.ID+"_created", operatorID, contact.ID, payload, contact.CreatedAt); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) UpsertBillingProfile(ctx context.Context, profile domain.BillingProfile, operatorID string) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, `INSERT INTO billing_profiles (id, customer_id, invoice_title, tax_identifier, billing_email, address, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON DUPLICATE KEY UPDATE invoice_title=VALUES(invoice_title), tax_identifier=VALUES(tax_identifier), billing_email=VALUES(billing_email), address=VALUES(address), updated_at=VALUES(updated_at)`, profile.ID, profile.CustomerID, profile.InvoiceTitle, profile.TaxIdentifier, profile.BillingEmail, profile.Address, profile.CreatedAt, profile.UpdatedAt); err != nil {
		return err
	}
	payload, _ := json.Marshal(profile)
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events (id, operator_id, action, resource_type, resource_id, payload_json, created_at) VALUES (?, ?, 'billing_profile.upserted', 'billing_profile', ?, ?, ?)`, profile.ID+"_upserted_"+profile.UpdatedAt.Format("150405.000000"), operatorID, profile.ID, payload, profile.UpdatedAt); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) Overview(ctx context.Context) (domain.Overview, error) {
	var overview domain.Overview
	err := store.db.QueryRowContext(ctx, `SELECT COUNT(*),
        COALESCE(SUM(CASE WHEN status = 'active' THEN 1 ELSE 0 END), 0),
        COALESCE(SUM(CASE WHEN status = 'lead' THEN 1 ELSE 0 END), 0)
        FROM customers`).Scan(&overview.CustomersTotal, &overview.CustomersActive, &overview.CustomersLeads)
	if err != nil {
		return overview, err
	}
	err = store.db.QueryRowContext(ctx, `SELECT
		(SELECT COUNT(*) FROM commercial_plan_heads),
		(SELECT COUNT(*) FROM commercial_orders WHERE status IN ('pending_payment', 'fulfillment_pending')),
		(SELECT COUNT(*) FROM commercial_free_distributions WHERE status = 'issued'),
		(SELECT COUNT(*) FROM commercial_paid_fulfillments WHERE status IN ('approved', 'prepared')),
		(SELECT COUNT(*) FROM commercial_paid_fulfillments WHERE status = 'issued'),
		(SELECT COUNT(*) FROM release_artifacts),
		(SELECT COUNT(*) FROM backup_history WHERE status IN ('failed', 'consistency_failed'))`).Scan(
		&overview.PlansTotal, &overview.OrdersPending, &overview.FreeDistributionsIssued,
		&overview.PaidFulfillmentsPending, &overview.PaidLicensesIssued, &overview.ReleaseArtifactsTotal, &overview.BackupsFailed)
	return overview, err
}

func (store *Store) CreatePlan(ctx context.Context, plan domain.Plan, operatorID string) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	now := plan.CreatedAt
	if _, err = tx.ExecContext(ctx, `INSERT INTO products (id, code, name, status, created_at, updated_at)
		VALUES ('product_aster_team', 'aster-team', 'Aster Team', 'active', ?, ?)
		ON DUPLICATE KEY UPDATE updated_at = updated_at`, now, now); err != nil {
		return err
	}
	features, err := json.Marshal(plan.Features)
	if err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO plans
		(id, product_id, code, name, edition, status, features_json, transfer_limit, member_seats_limit,
		 seat_over_limit_grace_days, minimum_version, created_at, updated_at)
		VALUES (?, 'product_aster_team', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
		plan.ID, plan.Code, plan.Name, plan.Edition, plan.Status, features, plan.TransferLimit, plan.MemberSeatsLimit,
		plan.SeatOverLimitGraceDays, plan.MinimumVersion, now, now); err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO price_versions
		(id, plan_id, version_no, currency, billing_cycle, base_amount_minor, included_member_seats,
		 additional_member_seat_minor, tax_mode, published_at, created_at)
		VALUES (?, ?, 1, ?, ?, ?, ?, ?, ?, ?, ?)`, plan.PriceVersionID, plan.ID, plan.Currency, plan.BillingCycle,
		plan.BaseAmountMinor, plan.IncludedMemberSeats, plan.AdditionalMemberSeatMinor, plan.TaxMode, now, now); err != nil {
		return err
	}
	payload, _ := json.Marshal(plan)
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events
		(id, operator_id, action, resource_type, resource_id, payload_json, created_at)
		VALUES (?, ?, 'plan.created', 'plan', ?, ?, ?)`, plan.ID+"_created", operatorID, plan.ID, payload, now); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) PublishPlanPrice(ctx context.Context, price domain.PriceVersionRecord, operatorID string) (domain.PriceVersionRecord, error) {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return price, err
	}
	defer tx.Rollback()
	var status string
	if err = tx.QueryRowContext(ctx, `SELECT status FROM plans WHERE id=? FOR UPDATE`, price.PlanID).Scan(&status); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return price, application.ErrNotFound
		}
		return price, err
	}
	if status != "active" {
		return price, fmt.Errorf("%w: archived plan cannot receive a price version", application.ErrValidation)
	}
	if err = tx.QueryRowContext(ctx, `SELECT COALESCE(MAX(version_no),0)+1 FROM price_versions WHERE plan_id=?`, price.PlanID).Scan(&price.VersionNo); err != nil {
		return price, err
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO price_versions (id, plan_id, version_no, currency, billing_cycle, base_amount_minor, included_member_seats, additional_member_seat_minor, tax_mode, published_at, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`, price.ID, price.PlanID, price.VersionNo, price.Currency, price.BillingCycle, price.BaseAmountMinor, price.IncludedMemberSeats, price.AdditionalMemberSeatMinor, price.TaxMode, price.PublishedAt, price.CreatedAt); err != nil {
		return price, err
	}
	if _, err = tx.ExecContext(ctx, `UPDATE plans SET updated_at=? WHERE id=?`, price.CreatedAt, price.PlanID); err != nil {
		return price, err
	}
	payload, _ := json.Marshal(price)
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events (id, operator_id, action, resource_type, resource_id, payload_json, created_at) VALUES (?, ?, 'plan.price_published', 'plan', ?, ?, ?)`, price.ID+"_audit", operatorID, price.PlanID, payload, price.CreatedAt); err != nil {
		return price, err
	}
	if err = tx.Commit(); err != nil {
		return price, err
	}
	return price, nil
}

func (store *Store) ListPlans(ctx context.Context, limit int) ([]domain.Plan, error) {
	return listPlans(ctx, store.db, limit)
}

func listPlans(ctx context.Context, database queryContext, limit int) ([]domain.Plan, error) {
	rows, err := database.QueryContext(ctx, `SELECT p.id, pr.code, p.code, p.name, p.edition, p.status, p.features_json,
		p.transfer_limit, p.member_seats_limit, p.seat_over_limit_grace_days,
		p.minimum_version,
		pv.id, pv.currency, pv.billing_cycle, pv.base_amount_minor, pv.included_member_seats,
		pv.additional_member_seat_minor, pv.tax_mode, p.created_at, p.updated_at
		FROM plans p JOIN products pr ON pr.id = p.product_id
		JOIN price_versions pv ON pv.plan_id = p.id
		WHERE pv.version_no = (SELECT MAX(latest.version_no) FROM price_versions latest WHERE latest.plan_id = p.id)
		ORDER BY p.created_at DESC LIMIT ?`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := make([]domain.Plan, 0, limit)
	for rows.Next() {
		var item domain.Plan
		var features []byte
		if err := rows.Scan(&item.ID, &item.ProductCode, &item.Code, &item.Name, &item.Edition, &item.Status, &features,
			&item.TransferLimit, &item.MemberSeatsLimit, &item.SeatOverLimitGraceDays, &item.MinimumVersion,
			&item.PriceVersionID, &item.Currency, &item.BillingCycle, &item.BaseAmountMinor,
			&item.IncludedMemberSeats, &item.AdditionalMemberSeatMinor, &item.TaxMode, &item.CreatedAt, &item.UpdatedAt); err != nil {
			return nil, err
		}
		if err := json.Unmarshal(features, &item.Features); err != nil {
			return nil, err
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func (store *Store) CreateOrder(ctx context.Context, order domain.Order, operatorID string) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	var planName, priceID, currency string
	var amount int64
	err = tx.QueryRowContext(ctx, `SELECT p.name, pv.id, pv.currency, pv.base_amount_minor
		FROM plans p JOIN price_versions pv ON pv.plan_id = p.id
		WHERE p.id = ? AND p.status = 'active' ORDER BY pv.version_no DESC LIMIT 1 FOR UPDATE`, order.PlanID).Scan(&planName, &priceID, &currency, &amount)
	if err != nil {
		return err
	}
	var customerName string
	if err = tx.QueryRowContext(ctx, "SELECT name FROM customers WHERE id = ? AND status <> 'inactive'", order.CustomerID).Scan(&customerName); err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO orders
		(id, customer_id, plan_id, price_version_id, contract_ref, status, amount_minor, currency,
		 starts_at, ends_at, notes, created_by, created_at, updated_at)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`, order.ID, order.CustomerID, order.PlanID, priceID,
		order.ContractRef, order.Status, amount, currency, order.StartsAt, order.EndsAt, order.Notes, operatorID,
		order.CreatedAt, order.UpdatedAt); err != nil {
		return err
	}
	snapshot, _ := json.Marshal(map[string]any{"plan_id": order.PlanID, "plan_name": planName, "price_version_id": priceID, "amount_minor": amount, "currency": currency})
	if _, err = tx.ExecContext(ctx, `INSERT INTO order_items
		(id, order_id, description, quantity, unit_amount_minor, price_snapshot_json, created_at)
		VALUES (?, ?, ?, 1, ?, ?, ?)`, order.ID+"_item_1", order.ID, planName, amount, snapshot, order.CreatedAt); err != nil {
		return err
	}
	payload, _ := json.Marshal(map[string]any{"customer_id": order.CustomerID, "plan_id": order.PlanID, "contract_ref": order.ContractRef, "amount_minor": amount, "currency": currency})
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events
		(id, operator_id, action, resource_type, resource_id, payload_json, created_at)
		VALUES (?, ?, 'order.created', 'order', ?, ?, ?)`, order.ID+"_created", operatorID, order.ID, payload, order.CreatedAt); err != nil {
		return err
	}
	return tx.Commit()
}

const orderSelect = `SELECT o.id, o.customer_id, c.name, o.plan_id, p.name, o.price_version_id, o.contract_ref,
	o.status, o.amount_minor, o.currency, o.starts_at, o.ends_at, o.notes, o.created_at, o.updated_at, pc.confirmed_at
	FROM orders o JOIN customers c ON c.id = o.customer_id JOIN plans p ON p.id = o.plan_id
	LEFT JOIN offline_payment_confirmations pc ON pc.order_id = o.id`

func (store *Store) ListOrders(ctx context.Context, limit int) ([]domain.Order, error) {
	return listOrders(ctx, store.db, limit)
}

func listOrders(ctx context.Context, database queryContext, limit int) ([]domain.Order, error) {
	rows, err := database.QueryContext(ctx, orderSelect+" ORDER BY o.created_at DESC LIMIT ?", limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := make([]domain.Order, 0, limit)
	for rows.Next() {
		item, err := scanOrder(rows)
		if err != nil {
			return nil, err
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func (store *Store) ConfirmOfflinePayment(ctx context.Context, orderID string, input domain.OfflinePaymentInput, operatorID string, now time.Time) (domain.Order, error) {
	var last error
	for attempt := 0; attempt < 3; attempt++ {
		order, err := store.confirmOfflinePaymentOnce(ctx, orderID, input, operatorID, now)
		if err == nil || !retryableMariaDBTransaction(err) {
			return order, err
		}
		last = err
		delay := time.Duration(attempt+1) * 20 * time.Millisecond
		timer := time.NewTimer(delay)
		select {
		case <-ctx.Done():
			timer.Stop()
			return domain.Order{}, ctx.Err()
		case <-timer.C:
		}
	}
	return domain.Order{}, last
}

func (store *Store) confirmOfflinePaymentOnce(ctx context.Context, orderID string, input domain.OfflinePaymentInput, operatorID string, now time.Time) (domain.Order, error) {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return domain.Order{}, err
	}
	defer tx.Rollback()
	var existingOrder string
	err = tx.QueryRowContext(ctx, "SELECT order_id FROM offline_payment_confirmations WHERE operation_id = ?", input.OperationID).Scan(&existingOrder)
	if err == nil {
		if existingOrder != orderID {
			return domain.Order{}, errors.New("payment operation ID was already used for another order")
		}
		if err := tx.Commit(); err != nil {
			return domain.Order{}, err
		}
		return store.getOrder(ctx, orderID)
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return domain.Order{}, err
	}
	var status, currency string
	var amount int64
	if err = tx.QueryRowContext(ctx, "SELECT status, amount_minor, currency FROM orders WHERE id = ? FOR UPDATE", orderID).Scan(&status, &amount, &currency); err != nil {
		return domain.Order{}, err
	}
	if status != "pending_payment" {
		return domain.Order{}, errors.New("order is not awaiting offline payment")
	}
	confirmationID := orderID + "_payment"
	if _, err = tx.ExecContext(ctx, `INSERT INTO offline_payment_confirmations
		(id, operation_id, order_id, payment_reference, amount_minor, currency, confirmed_by, reviewed_by, confirmed_at, notes)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`, confirmationID, input.OperationID, orderID, input.PaymentReference,
		amount, currency, operatorID, operatorID, now, input.Notes); err != nil {
		return domain.Order{}, err
	}
	if _, err = tx.ExecContext(ctx, "UPDATE orders SET status = 'fulfillment_pending', updated_at = ? WHERE id = ? AND status = 'pending_payment'", now, orderID); err != nil {
		return domain.Order{}, err
	}
	payload, _ := json.Marshal(map[string]any{"operation_id": input.OperationID, "payment_reference": input.PaymentReference, "amount_minor": amount, "currency": currency})
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events
		(id, operator_id, action, resource_type, resource_id, payload_json, created_at)
		VALUES (?, ?, 'order.offline_payment_confirmed', 'order', ?, ?, ?)`, input.OperationID+"_audit", operatorID, orderID, payload, now); err != nil {
		return domain.Order{}, err
	}
	if err = tx.Commit(); err != nil {
		return domain.Order{}, err
	}
	return store.getOrder(ctx, orderID)
}

func retryableMariaDBTransaction(err error) bool {
	var mysqlError *mysqldriver.MySQLError
	return errors.As(err, &mysqlError) && (mysqlError.Number == 1213 || mysqlError.Number == 1205)
}

func (store *Store) getOrder(ctx context.Context, orderID string) (domain.Order, error) {
	return scanOrder(store.db.QueryRowContext(ctx, orderSelect+" WHERE o.id = ?", orderID))
}

type rowScanner interface{ Scan(...any) error }

func scanOrder(row rowScanner) (domain.Order, error) {
	var item domain.Order
	var paidAt sql.NullTime
	err := row.Scan(&item.ID, &item.CustomerID, &item.CustomerName, &item.PlanID, &item.PlanName, &item.PriceVersionID,
		&item.ContractRef, &item.Status, &item.AmountMinor, &item.Currency, &item.StartsAt, &item.EndsAt, &item.Notes,
		&item.CreatedAt, &item.UpdatedAt, &paidAt)
	if paidAt.Valid {
		item.PaidAt = &paidAt.Time
	}
	return item, err
}

func (store *Store) CreateTrial(ctx context.Context, trial domain.Trial, operatorID string) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	var prior int
	if err = tx.QueryRowContext(ctx, `SELECT COUNT(*) FROM trials WHERE customer_id = ? AND status IN ('approved', 'active')`, trial.CustomerID).Scan(&prior); err != nil {
		return err
	}
	if prior > 0 {
		return errors.New("customer already has an active or approved trial")
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO trials
		(id, customer_id, plan_id, status, starts_at, ends_at, member_seats, transfer_limit,
		 approval_reason, approved_by, created_at, updated_at)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`, trial.ID, trial.CustomerID, trial.PlanID, trial.Status,
		trial.StartsAt, trial.EndsAt, trial.MemberSeats, trial.TransferLimit, trial.ApprovalReason,
		operatorID, trial.CreatedAt, trial.UpdatedAt); err != nil {
		return err
	}
	payload, _ := json.Marshal(trial)
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events
		(id, operator_id, action, resource_type, resource_id, payload_json, created_at)
		VALUES (?, ?, 'trial.approved', 'trial', ?, ?, ?)`, trial.ID+"_approved", operatorID, trial.ID, payload, trial.CreatedAt); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) ListTrials(ctx context.Context, limit int) ([]domain.Trial, error) {
	return listTrials(ctx, store.db, limit)
}

func listTrials(ctx context.Context, database queryContext, limit int) ([]domain.Trial, error) {
	rows, err := database.QueryContext(ctx, `SELECT t.id, t.customer_id, c.name, t.plan_id, p.name, t.status,
		t.starts_at, t.ends_at, t.member_seats, t.transfer_limit, t.approval_reason, t.created_at, t.updated_at
		FROM trials t JOIN customers c ON c.id = t.customer_id JOIN plans p ON p.id = t.plan_id
		ORDER BY t.created_at DESC LIMIT ?`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := make([]domain.Trial, 0, limit)
	for rows.Next() {
		var item domain.Trial
		if err := rows.Scan(&item.ID, &item.CustomerID, &item.CustomerName, &item.PlanID, &item.PlanName, &item.Status,
			&item.StartsAt, &item.EndsAt, &item.MemberSeats, &item.TransferLimit,
			&item.ApprovalReason, &item.CreatedAt, &item.UpdatedAt); err != nil {
			return nil, err
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func (store *Store) CreateRefundNote(ctx context.Context, note domain.RefundNote, operatorID string) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	var amount int64
	var status string
	if err = tx.QueryRowContext(ctx, `SELECT amount_minor, status FROM orders WHERE id=? FOR UPDATE`, note.OrderID).Scan(&amount, &status); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return application.ErrNotFound
		}
		return err
	}
	if note.AmountMinor > amount || (status != "fulfilled" && status != "fulfillment_pending") {
		return fmt.Errorf("%w: refund amount or order status is invalid", application.ErrValidation)
	}
	rows, err := tx.QueryContext(ctx, `SELECT amount_minor FROM refund_notes WHERE order_id=? FOR UPDATE`, note.OrderID)
	if err != nil {
		return err
	}
	var refunded int64
	for rows.Next() {
		var prior int64
		if err := rows.Scan(&prior); err != nil {
			rows.Close()
			return err
		}
		refunded += prior
	}
	if err := rows.Close(); err != nil {
		return err
	}
	if refunded+note.AmountMinor > amount {
		return fmt.Errorf("%w: cumulative refund exceeds order amount", application.ErrValidation)
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO refund_notes (id, order_id, amount_minor, reason, operator_id, created_at) VALUES (?, ?, ?, ?, ?, ?)`, note.ID, note.OrderID, note.AmountMinor, note.Reason, operatorID, note.CreatedAt); err != nil {
		return err
	}
	if refunded+note.AmountMinor == amount {
		if _, err = tx.ExecContext(ctx, `UPDATE orders SET status='refunded', updated_at=? WHERE id=?`, note.CreatedAt, note.OrderID); err != nil {
			return err
		}
	}
	payload, _ := json.Marshal(note)
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events (id, operator_id, action, resource_type, resource_id, payload_json, created_at) VALUES (?, ?, 'order.refund_recorded', 'order', ?, ?, ?)`, note.ID+"_audit", operatorID, note.OrderID, payload, note.CreatedAt); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) ListRefundNotes(ctx context.Context, orderID string) ([]domain.RefundNote, error) {
	return listRefundNotes(ctx, store.db, orderID)
}

func listRefundNotes(ctx context.Context, database queryContext, orderID string) ([]domain.RefundNote, error) {
	query := `SELECT id, order_id, amount_minor, reason, operator_id, created_at FROM refund_notes`
	args := []any{}
	if orderID != "" {
		query += " WHERE order_id=?"
		args = append(args, orderID)
	}
	query += " ORDER BY created_at DESC"
	rows, err := database.QueryContext(ctx, query, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := []domain.RefundNote{}
	for rows.Next() {
		var item domain.RefundNote
		if err := rows.Scan(&item.ID, &item.OrderID, &item.AmountMinor, &item.Reason, &item.OperatorID, &item.CreatedAt); err != nil {
			return nil, err
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func (store *Store) ExtendTrial(ctx context.Context, trialID string, endsAt time.Time, reason, operatorID string, now time.Time) (domain.Trial, error) {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return domain.Trial{}, err
	}
	defer tx.Rollback()
	var before time.Time
	var status string
	if err = tx.QueryRowContext(ctx, `SELECT ends_at, status FROM trials WHERE id=? FOR UPDATE`, trialID).Scan(&before, &status); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return domain.Trial{}, application.ErrNotFound
		}
		return domain.Trial{}, err
	}
	if (status != "approved" && status != "active") || !endsAt.After(before) || endsAt.Sub(before) > 30*24*time.Hour || endsAt.Sub(now) > 60*24*time.Hour {
		return domain.Trial{}, fmt.Errorf("%w: trial extension exceeds policy", application.ErrValidation)
	}
	if _, err = tx.ExecContext(ctx, `UPDATE trials SET ends_at=?, updated_at=? WHERE id=?`, endsAt, now, trialID); err != nil {
		return domain.Trial{}, err
	}
	var policyJSON []byte
	var licenseID string
	policyErr := tx.QueryRowContext(ctx, `SELECT license_id, policy_json FROM license_records WHERE source_type='trial' AND source_id=? FOR UPDATE`, trialID).Scan(&licenseID, &policyJSON)
	if policyErr == nil {
		var policy ports.LicensePolicy
		if err := json.Unmarshal(policyJSON, &policy); err != nil {
			return domain.Trial{}, err
		}
		policy.ValidUntil = endsAt
		request, err := json.Marshal(policy)
		if err != nil {
			return domain.Trial{}, err
		}
		if _, err = tx.ExecContext(ctx, `UPDATE license_records SET policy_json=?, updated_at=? WHERE license_id=?`, request, now, licenseID); err != nil {
			return domain.Trial{}, err
		}
	} else if !errors.Is(policyErr, sql.ErrNoRows) {
		return domain.Trial{}, policyErr
	}
	payload, _ := json.Marshal(map[string]any{"before_ends_at": before, "after_ends_at": endsAt, "reason": reason})
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events (id, operator_id, action, resource_type, resource_id, payload_json, created_at) VALUES (?, ?, 'trial.extended', 'trial', ?, ?, ?)`, trialID+"_extended_"+now.Format("150405.000000"), operatorID, trialID, payload, now); err != nil {
		return domain.Trial{}, err
	}
	if err = tx.Commit(); err != nil {
		return domain.Trial{}, err
	}
	return store.getTrial(ctx, trialID)
}

func (store *Store) getTrial(ctx context.Context, trialID string) (domain.Trial, error) {
	var item domain.Trial
	err := store.db.QueryRowContext(ctx, `SELECT t.id, t.customer_id, c.name, t.plan_id, p.name, t.status, t.starts_at, t.ends_at, t.member_seats, t.transfer_limit, t.approval_reason, t.created_at, t.updated_at FROM trials t JOIN customers c ON c.id=t.customer_id JOIN plans p ON p.id=t.plan_id WHERE t.id=?`, trialID).Scan(&item.ID, &item.CustomerID, &item.CustomerName, &item.PlanID, &item.PlanName, &item.Status, &item.StartsAt, &item.EndsAt, &item.MemberSeats, &item.TransferLimit, &item.ApprovalReason, &item.CreatedAt, &item.UpdatedAt)
	if errors.Is(err, sql.ErrNoRows) {
		return item, application.ErrNotFound
	}
	return item, err
}

func (store *Store) CreateRiskNote(ctx context.Context, note domain.RiskNote, operatorID string) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, `INSERT INTO risk_notes (id, trial_id, risk_level, note, operator_id, created_at) VALUES (?, ?, ?, ?, ?, ?)`, note.ID, note.TrialID, note.RiskLevel, note.Note, operatorID, note.CreatedAt); err != nil {
		return err
	}
	payload, _ := json.Marshal(note)
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events (id, operator_id, action, resource_type, resource_id, payload_json, created_at) VALUES (?, ?, 'trial.risk_noted', 'trial', ?, ?, ?)`, note.ID+"_audit", operatorID, note.TrialID, payload, note.CreatedAt); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) ListRiskNotes(ctx context.Context, trialID string) ([]domain.RiskNote, error) {
	return listRiskNotes(ctx, store.db, trialID)
}

func listRiskNotes(ctx context.Context, database queryContext, trialID string) ([]domain.RiskNote, error) {
	query := `SELECT id, trial_id, risk_level, note, operator_id, created_at FROM risk_notes`
	args := []any{}
	if trialID != "" {
		query += " WHERE trial_id=?"
		args = append(args, trialID)
	}
	query += " ORDER BY created_at DESC"
	rows, err := database.QueryContext(ctx, query, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := []domain.RiskNote{}
	for rows.Next() {
		var item domain.RiskNote
		if err := rows.Scan(&item.ID, &item.TrialID, &item.RiskLevel, &item.Note, &item.OperatorID, &item.CreatedAt); err != nil {
			return nil, err
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func (store *Store) ListBackupHistory(ctx context.Context, limit int) ([]domain.BackupRecord, error) {
	return listBackupHistory(ctx, store.db, limit)
}

func listBackupHistory(ctx context.Context, database queryContext, limit int) ([]domain.BackupRecord, error) {
	rows, err := database.QueryContext(ctx, `SELECT id, kind, object_ref, sha256, size_bytes, status, operator_id, created_at, verified_at FROM backup_history ORDER BY created_at DESC LIMIT ?`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := []domain.BackupRecord{}
	for rows.Next() {
		var item domain.BackupRecord
		var verified sql.NullTime
		if err := rows.Scan(&item.ID, &item.Kind, &item.ObjectRef, &item.SHA256, &item.SizeBytes, &item.Status, &item.OperatorID, &item.CreatedAt, &verified); err != nil {
			return nil, err
		}
		if verified.Valid {
			item.VerifiedAt = &verified.Time
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func (store *Store) ExportOperations(ctx context.Context, operatorID string, now time.Time) (domain.OperationsExport, error) {
	payload, _ := json.Marshal(map[string]any{"schema_version": "aster.operations-export.v2", "redacted": true})
	digest := sha256.Sum256([]byte(operatorID + now.Format(time.RFC3339Nano)))
	auditID := "export_" + hex.EncodeToString(digest[:])
	export := domain.OperationsExport{SchemaVersion: "aster.operations-export.v2", ExportedAt: now}
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelRepeatableRead, ReadOnly: true})
	if err != nil {
		return export, err
	}
	defer tx.Rollback()
	if export.Customers, err = listCustomers(ctx, tx, "", 1_000_000); err != nil {
		return export, err
	}
	if export.Contacts, err = listAllContacts(ctx, tx); err != nil {
		return export, err
	}
	if export.Billing, err = listAllBillingProfiles(ctx, tx); err != nil {
		return export, err
	}
	if export.CommercialRecords, err = listCommercialExportRecords(ctx, tx); err != nil {
		return export, err
	}
	if export.ReleaseArtifacts, err = listReleaseArtifacts(ctx, tx, 1_000_000); err != nil {
		return export, err
	}
	if export.AuditEvents, err = listAuditEvents(ctx, tx, 1_000_000); err != nil {
		return export, err
	}
	if export.Backups, err = listBackupHistory(ctx, tx, 1_000_000); err != nil {
		return export, err
	}
	if err = tx.Commit(); err != nil {
		return export, err
	}
	if _, err := store.db.ExecContext(ctx, `INSERT INTO audit_events (id, operator_id, action, resource_type, resource_id, payload_json, created_at) VALUES (?, ?, 'operations.exported', 'operations', 'business-data', ?, ?)`, auditID, operatorID, payload, now); err != nil {
		return export, err
	}
	return export, nil
}

func listAllContacts(ctx context.Context, database queryContext) ([]domain.Contact, error) {
	rows, err := database.QueryContext(ctx, `SELECT id, customer_id, name, email, phone, wechat, role_title, is_primary, created_at, updated_at FROM contacts ORDER BY created_at`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := []domain.Contact{}
	for rows.Next() {
		var item domain.Contact
		if err := rows.Scan(&item.ID, &item.CustomerID, &item.Name, &item.Email, &item.Phone, &item.WeChat, &item.RoleTitle, &item.IsPrimary, &item.CreatedAt, &item.UpdatedAt); err != nil {
			return nil, err
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func listAllBillingProfiles(ctx context.Context, database queryContext) ([]domain.BillingProfile, error) {
	rows, err := database.QueryContext(ctx, `SELECT id, customer_id, invoice_title, tax_identifier, billing_email, address, created_at, updated_at FROM billing_profiles ORDER BY created_at`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := []domain.BillingProfile{}
	for rows.Next() {
		var item domain.BillingProfile
		if err := rows.Scan(&item.ID, &item.CustomerID, &item.InvoiceTitle, &item.TaxIdentifier, &item.BillingEmail, &item.Address, &item.CreatedAt, &item.UpdatedAt); err != nil {
			return nil, err
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func listAllProducts(ctx context.Context, database queryContext) ([]domain.ProductRecord, error) {
	rows, err := database.QueryContext(ctx, `SELECT id, code, name, status, created_at, updated_at FROM products ORDER BY created_at`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := []domain.ProductRecord{}
	for rows.Next() {
		var item domain.ProductRecord
		if err := rows.Scan(&item.ID, &item.Code, &item.Name, &item.Status, &item.CreatedAt, &item.UpdatedAt); err != nil {
			return nil, err
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func listAllPriceVersions(ctx context.Context, database queryContext) ([]domain.PriceVersionRecord, error) {
	rows, err := database.QueryContext(ctx, `SELECT id, plan_id, version_no, currency, billing_cycle, base_amount_minor, included_member_seats, additional_member_seat_minor, tax_mode, published_at, created_at FROM price_versions ORDER BY created_at`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := []domain.PriceVersionRecord{}
	for rows.Next() {
		var item domain.PriceVersionRecord
		if err := rows.Scan(&item.ID, &item.PlanID, &item.VersionNo, &item.Currency, &item.BillingCycle, &item.BaseAmountMinor, &item.IncludedMemberSeats, &item.AdditionalMemberSeatMinor, &item.TaxMode, &item.PublishedAt, &item.CreatedAt); err != nil {
			return nil, err
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func listAllOrderItems(ctx context.Context, database queryContext) ([]domain.OrderItemRecord, error) {
	rows, err := database.QueryContext(ctx, `SELECT id, order_id, description, quantity, unit_amount_minor, price_snapshot_json, created_at FROM order_items ORDER BY created_at`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := []domain.OrderItemRecord{}
	for rows.Next() {
		var item domain.OrderItemRecord
		var snapshot []byte
		if err := rows.Scan(&item.ID, &item.OrderID, &item.Description, &item.Quantity, &item.UnitAmountMinor, &snapshot, &item.CreatedAt); err != nil {
			return nil, err
		}
		item.PriceSnapshot = json.RawMessage(snapshot)
		items = append(items, item)
	}
	return items, rows.Err()
}

func listAllPaymentConfirmations(ctx context.Context, database queryContext) ([]domain.OfflinePaymentConfirmation, error) {
	rows, err := database.QueryContext(ctx, `SELECT id, operation_id, order_id, payment_reference, amount_minor, currency, confirmed_by, reviewed_by, confirmed_at, notes FROM offline_payment_confirmations ORDER BY confirmed_at`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := []domain.OfflinePaymentConfirmation{}
	for rows.Next() {
		var item domain.OfflinePaymentConfirmation
		if err := rows.Scan(&item.ID, &item.OperationID, &item.OrderID, &item.PaymentReference, &item.AmountMinor, &item.Currency, &item.ConfirmedBy, &item.ReviewedBy, &item.ConfirmedAt, &item.Notes); err != nil {
			return nil, err
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func listAllLicenseRecords(ctx context.Context, database queryContext) ([]domain.LocalLicenseRecord, error) {
	rows, err := database.QueryContext(ctx, `SELECT license_id, customer_id, source_type, source_id, customer_ref, policy_json, status, transfer_limit, transfer_count, created_at, updated_at FROM license_records ORDER BY created_at`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := []domain.LocalLicenseRecord{}
	for rows.Next() {
		var item domain.LocalLicenseRecord
		var policy []byte
		if err := rows.Scan(&item.LicenseID, &item.CustomerID, &item.SourceType, &item.SourceID, &item.CustomerRef, &policy, &item.Status, &item.TransferLimit, &item.TransferCount, &item.CreatedAt, &item.UpdatedAt); err != nil {
			return nil, err
		}
		item.Policy = json.RawMessage(policy)
		items = append(items, item)
	}
	return items, rows.Err()
}

func listAllLicenseIssuances(ctx context.Context, database queryContext) ([]domain.LocalLicenseIssuance, error) {
	rows, err := database.QueryContext(ctx, `SELECT id, license_id, request_id, installation_id, machine_fingerprint_sha256,
		transfer_sequence, document_json, sha256, issued_by, issued_at, expires_at FROM license_issuances ORDER BY issued_at`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := []domain.LocalLicenseIssuance{}
	for rows.Next() {
		var item domain.LocalLicenseIssuance
		if err := rows.Scan(&item.ID, &item.LicenseID, &item.RequestID, &item.InstallationID,
			&item.MachineFingerprintSHA256, &item.TransferSequence, &item.Document, &item.SHA256,
			&item.IssuedBy, &item.IssuedAt, &item.ExpiresAt); err != nil {
			return nil, err
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func (store *Store) ListAuditEvents(ctx context.Context, limit int) ([]domain.AuditEvent, error) {
	return listAuditEvents(ctx, store.db, limit)
}

func listAuditEvents(ctx context.Context, database queryContext, limit int) ([]domain.AuditEvent, error) {
	rows, err := database.QueryContext(ctx, `SELECT id, operator_id, action, resource_type, resource_id, payload_json, created_at
		FROM audit_events ORDER BY created_at DESC LIMIT ?`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := make([]domain.AuditEvent, 0, limit)
	for rows.Next() {
		var item domain.AuditEvent
		var payload []byte
		if err := rows.Scan(&item.ID, &item.OperatorID, &item.Action, &item.ResourceType, &item.ResourceID, &payload, &item.CreatedAt); err != nil {
			return nil, err
		}
		item.Payload = json.RawMessage(payload)
		items = append(items, item)
	}
	return items, rows.Err()
}

func (store *Store) FulfillmentCustomer(ctx context.Context, sourceType, sourceID string) (string, error) {
	var customerID string
	var err error
	switch sourceType {
	case "order":
		err = store.db.QueryRowContext(ctx, "SELECT customer_id FROM orders WHERE id = ?", sourceID).Scan(&customerID)
	case "trial":
		err = store.db.QueryRowContext(ctx, "SELECT customer_id FROM trials WHERE id = ?", sourceID).Scan(&customerID)
	default:
		return "", errors.New("unsupported fulfillment source")
	}
	return customerID, err
}

func (store *Store) PrepareSourceFulfillment(ctx context.Context, sourceType, sourceID string, input domain.FulfillmentInput, customerRef, operatorID string, now time.Time) (ports.LicenseRecord, error) {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return ports.LicenseRecord{}, err
	}
	defer tx.Rollback()

	var existingCustomerID string
	var existingPolicy []byte
	var existing ports.LicenseRecord
	err = tx.QueryRowContext(ctx, `SELECT customer_id, policy_json, status, transfer_limit, transfer_count, created_at, updated_at
		FROM license_records WHERE source_type=? AND source_id=? FOR UPDATE`, sourceType, sourceID).
		Scan(&existingCustomerID, &existingPolicy, &existing.Status, &existing.TransferLimit, &existing.TransferCount, &existing.CreatedAt, &existing.UpdatedAt)
	if err == nil {
		if json.Unmarshal(existingPolicy, &existing.LicensePolicy) != nil || existing.LicenseID != input.LicenseID ||
			existing.OperationID != input.OperationID || existing.CustomerRef != customerRef {
			return ports.LicenseRecord{}, errors.New("fulfillment source is already licensed")
		}
		if err := tx.Commit(); err != nil {
			return ports.LicenseRecord{}, err
		}
		return existing, nil
	}
	if !errors.Is(err, sql.ErrNoRows) {
		return ports.LicenseRecord{}, err
	}

	var customerID, status, edition, minimumVersion string
	var endsAt time.Time
	var featuresJSON []byte
	var transferLimit, memberSeats, graceDays int
	switch sourceType {
	case "order":
		err = tx.QueryRowContext(ctx, `SELECT o.customer_id, o.status, o.ends_at, p.edition, p.features_json,
			p.transfer_limit, p.member_seats_limit,
			p.seat_over_limit_grace_days, p.minimum_version
			FROM orders o JOIN plans p ON p.id=o.plan_id WHERE o.id=? FOR UPDATE`, sourceID).
			Scan(&customerID, &status, &endsAt, &edition, &featuresJSON, &transferLimit, &memberSeats,
				&graceDays, &minimumVersion)
		if err == nil && status != "fulfillment_pending" {
			return ports.LicenseRecord{}, errors.New("order payment has not been confirmed")
		}
	case "trial":
		err = tx.QueryRowContext(ctx, `SELECT t.customer_id, t.status, t.ends_at, p.edition, p.features_json,
			t.transfer_limit, t.member_seats,
			p.seat_over_limit_grace_days, p.minimum_version
			FROM trials t JOIN plans p ON p.id=t.plan_id WHERE t.id=? FOR UPDATE`, sourceID).
			Scan(&customerID, &status, &endsAt, &edition, &featuresJSON, &transferLimit, &memberSeats,
				&graceDays, &minimumVersion)
		if err == nil && status != "approved" {
			return ports.LicenseRecord{}, errors.New("trial is not approved")
		}
	default:
		return ports.LicenseRecord{}, errors.New("unsupported fulfillment source")
	}
	if err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return ports.LicenseRecord{}, application.ErrNotFound
		}
		return ports.LicenseRecord{}, err
	}
	if !endsAt.After(now) {
		return ports.LicenseRecord{}, errors.New("fulfillment source has expired")
	}
	var features []string
	if err := json.Unmarshal(featuresJSON, &features); err != nil {
		return ports.LicenseRecord{}, err
	}
	policy := ports.LicensePolicy{
		OperationID: input.OperationID, LicenseID: input.LicenseID, CustomerRef: customerRef,
		SourceType: sourceType, SourceID: sourceID, Product: "aster-team", Edition: edition, Features: features,
		Limits:         ports.LicenseLimits{MemberSeats: memberSeats, SeatOverLimitGraceDays: graceDays},
		MinimumVersion: minimumVersion, ValidUntil: endsAt.UTC(), TransferLimit: transferLimit,
	}
	policyJSON, err := json.Marshal(policy)
	if err != nil {
		return ports.LicenseRecord{}, err
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO license_records
		(license_id,operation_id,customer_id,source_type,source_id,customer_ref,policy_json,status,transfer_limit,transfer_count,created_at,updated_at)
		VALUES(?,?,?,?,?,?,?,'active',?,0,?,?)`, policy.LicenseID, policy.OperationID, customerID, sourceType,
		sourceID, customerRef, policyJSON, transferLimit, now, now); err != nil {
		return ports.LicenseRecord{}, err
	}
	if sourceType == "order" {
		if _, err = tx.ExecContext(ctx, `UPDATE orders SET status='fulfilled',updated_at=? WHERE id=?`, now, sourceID); err != nil {
			return ports.LicenseRecord{}, err
		}
	} else {
		if _, err = tx.ExecContext(ctx, `UPDATE trials SET status='active',updated_at=? WHERE id=?`, now, sourceID); err != nil {
			return ports.LicenseRecord{}, err
		}
	}
	payload, _ := json.Marshal(map[string]any{"operation_id": input.OperationID, "license_id": input.LicenseID, "source_type": sourceType, "source_id": sourceID})
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events
		(id,operator_id,action,resource_type,resource_id,payload_json,created_at)
		VALUES(?,?,'license.policy_created','license',?,?,?)`, operationAuditID(input.OperationID, "created"), operatorID,
		input.LicenseID, payload, now); err != nil {
		return ports.LicenseRecord{}, err
	}
	if err := tx.Commit(); err != nil {
		return ports.LicenseRecord{}, err
	}
	return ports.LicenseRecord{LicensePolicy: policy, Status: "active", TransferCount: 0, CreatedAt: now, UpdatedAt: now}, nil
}

func operationAuditID(operationID, phase string) string {
	digest := sha256.Sum256([]byte(operationID + "\x00" + phase))
	return "audit_" + hex.EncodeToString(digest[:28])
}

func (store *Store) CreateReleaseArtifact(ctx context.Context, release domain.ReleaseArtifact, operatorID string) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, `INSERT INTO release_artifacts
		(id, version, platform, architecture, object_key, sha256, release_manifest_sha256, size_bytes, signature_ref,
		 source_commit_sha, github_run_id, runtime_linkage, imported_by, created_at)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`, release.ID, release.Version, release.Platform, release.Architecture,
		release.ObjectKey, release.SHA256, release.ReleaseManifestSHA256, release.SizeBytes, release.SignatureRef,
		release.SourceCommitSHA, release.GitHubRunID, release.RuntimeLinkage, operatorID, release.CreatedAt); err != nil {
		return err
	}
	payload, _ := json.Marshal(release)
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events
		(id, operator_id, action, resource_type, resource_id, payload_json, created_at)
		VALUES (?, ?, 'release.imported', 'release_artifact', ?, ?, ?)`, release.ID+"_imported", operatorID, release.ID, payload, release.CreatedAt); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) ListReleaseArtifacts(ctx context.Context, limit int) ([]domain.ReleaseArtifact, error) {
	return listReleaseArtifacts(ctx, store.db, limit)
}

func listReleaseArtifacts(ctx context.Context, database queryContext, limit int) ([]domain.ReleaseArtifact, error) {
	rows, err := database.QueryContext(ctx, `SELECT id, version, platform, architecture, object_key, sha256, release_manifest_sha256, size_bytes,
		signature_ref, source_commit_sha, github_run_id, runtime_linkage, created_at
		FROM release_artifacts ORDER BY created_at DESC LIMIT ?`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := make([]domain.ReleaseArtifact, 0, limit)
	for rows.Next() {
		var item domain.ReleaseArtifact
		var sourceCommit sql.NullString
		var githubRunID sql.NullInt64
		if err := rows.Scan(&item.ID, &item.Version, &item.Platform, &item.Architecture, &item.ObjectKey, &item.SHA256,
			&item.ReleaseManifestSHA256, &item.SizeBytes, &item.SignatureRef, &sourceCommit, &githubRunID,
			&item.RuntimeLinkage, &item.CreatedAt); err != nil {
			return nil, err
		}
		if sourceCommit.Valid {
			item.SourceCommitSHA = &sourceCommit.String
		}
		if githubRunID.Valid {
			item.GitHubRunID = &githubRunID.Int64
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func (store *Store) GetReleaseArtifact(ctx context.Context, releaseID string) (domain.ReleaseArtifact, error) {
	var item domain.ReleaseArtifact
	var sourceCommit sql.NullString
	var githubRunID sql.NullInt64
	err := store.db.QueryRowContext(ctx, `SELECT id, version, platform, architecture, object_key, sha256, release_manifest_sha256,
		size_bytes, signature_ref, source_commit_sha, github_run_id, runtime_linkage, created_at
		FROM release_artifacts WHERE id = ?`, releaseID).Scan(&item.ID, &item.Version, &item.Platform, &item.Architecture,
		&item.ObjectKey, &item.SHA256, &item.ReleaseManifestSHA256, &item.SizeBytes, &item.SignatureRef, &sourceCommit,
		&githubRunID, &item.RuntimeLinkage, &item.CreatedAt)
	if sourceCommit.Valid {
		item.SourceCommitSHA = &sourceCommit.String
	}
	if githubRunID.Valid {
		item.GitHubRunID = &githubRunID.Int64
	}
	return item, err
}

func (store *Store) ListReleaseTasks(ctx context.Context, limit int) ([]domain.ReleaseTask, error) {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelRepeatableRead, ReadOnly: true})
	if err != nil {
		return nil, err
	}
	defer tx.Rollback()
	rows, err := tx.QueryContext(ctx, `SELECT t.id, t.version, t.mode, t.github_repository,
		t.workflow_file, t.source_ref, t.source_commit_sha, t.free_distribution_id, t.free_license_sha256, t.phase, t.status, t.error_code, t.retry_of_task_id, t.created_by,
		t.created_at, t.updated_at, t.started_at, t.completed_at,
		COALESCE((SELECT r.html_url FROM release_runs r WHERE r.release_task_id=t.id ORDER BY r.attempt DESC LIMIT 1), ''),
		(SELECT r.conclusion FROM release_runs r WHERE r.release_task_id=t.id ORDER BY r.attempt DESC LIMIT 1)
		FROM release_tasks t
		ORDER BY t.created_at DESC, t.id DESC LIMIT ?`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := make([]domain.ReleaseTask, 0, limit)
	for rows.Next() {
		item, scanErr := scanReleaseTaskSummary(rows)
		if scanErr != nil {
			return nil, scanErr
		}
		items = append(items, item)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	if err := rows.Close(); err != nil {
		return nil, err
	}
	ids := make([]string, len(items))
	indices := map[string]int{}
	for index := range items {
		ids[index] = items[index].ID
		indices[items[index].ID] = index
	}
	artifacts, err := loadReleaseTaskArtifacts(ctx, tx, ids)
	if err != nil {
		return nil, err
	}
	for _, artifact := range artifacts {
		if artifact.Target().Supported() {
			index := indices[artifact.ReleaseTaskID]
			items[index].Packages = append(items[index].Packages, artifact)
		}
	}
	return items, tx.Commit()
}

func scanReleaseTaskSummary(scanner rowScanner) (domain.ReleaseTask, error) {
	var item domain.ReleaseTask
	var freeDistributionID, freeLicenseSHA256, errorCode, retryOf, conclusion sql.NullString
	var startedAt, completedAt sql.NullTime
	err := scanner.Scan(&item.ID, &item.Version, &item.Mode, &item.GitHubRepository,
		&item.WorkflowFile, &item.SourceRef, &item.SourceCommitSHA, &freeDistributionID, &freeLicenseSHA256, &item.Phase, &item.Status, &errorCode, &retryOf,
		&item.CreatedBy, &item.CreatedAt, &item.UpdatedAt, &startedAt, &completedAt, &item.GitHubRunURL, &conclusion)
	if err != nil {
		return item, err
	}
	if errorCode.Valid {
		item.ErrorCode = &errorCode.String
	}
	if freeDistributionID.Valid {
		item.FreeDistributionID = freeDistributionID.String
	}
	if freeLicenseSHA256.Valid {
		item.FreeLicenseSHA256 = freeLicenseSHA256.String
	}
	if retryOf.Valid {
		item.RetryOfTaskID = &retryOf.String
	}
	if startedAt.Valid {
		item.StartedAt = &startedAt.Time
	}
	if completedAt.Valid {
		item.CompletedAt = &completedAt.Time
	}
	if conclusion.Valid {
		item.GitHubConclusion = &conclusion.String
	}
	item.Packages = []domain.ReleaseTaskArtifact{}
	return item, nil
}

func (store *Store) GetReleaseTaskDetail(ctx context.Context, taskID string) (domain.ReleaseTaskDetail, error) {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelRepeatableRead, ReadOnly: true})
	if err != nil {
		return domain.ReleaseTaskDetail{}, err
	}
	defer tx.Rollback()
	task, err := scanReleaseTask(tx.QueryRowContext(ctx, `SELECT id, version, mode, github_repository,
		workflow_file, source_ref, source_commit_sha, free_distribution_id, free_license_sha256, phase, status, error_code, retry_of_task_id, created_by,
		created_at, updated_at, started_at, completed_at FROM release_tasks WHERE id = ?`, taskID))
	if errors.Is(err, sql.ErrNoRows) {
		return domain.ReleaseTaskDetail{}, fmt.Errorf("%w: release task does not exist", application.ErrNotFound)
	}
	if err != nil {
		return domain.ReleaseTaskDetail{}, err
	}
	detail := domain.ReleaseTaskDetail{Task: task, Runs: []domain.ReleaseRun{}, Artifacts: []domain.ReleaseTaskArtifact{}}
	if err = loadReleaseRuns(ctx, tx, taskID, &detail); err != nil {
		return domain.ReleaseTaskDetail{}, err
	}
	if detail.Artifacts, err = loadReleaseTaskArtifacts(ctx, tx, []string{taskID}); err != nil {
		return domain.ReleaseTaskDetail{}, err
	}
	if err = tx.Commit(); err != nil {
		return domain.ReleaseTaskDetail{}, err
	}
	detail.Task.Packages = []domain.ReleaseTaskArtifact{}
	for _, artifact := range detail.Artifacts {
		if artifact.Target().Supported() {
			detail.Task.Packages = append(detail.Task.Packages, artifact)
		}
	}
	if len(detail.Runs) > 0 {
		detail.Task.GitHubRunURL = detail.Runs[0].HTMLURL
		detail.Task.GitHubConclusion = detail.Runs[0].Conclusion
	}
	return detail, nil
}

func (store *Store) CreateReleaseTask(ctx context.Context, task domain.ReleaseTask, operatorID string) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	dedupKey := task.Version + ":" + task.SourceCommitSHA
	_, err = tx.ExecContext(ctx, `INSERT INTO release_tasks
		(id, version, mode, github_repository, workflow_file, source_ref, source_commit_sha, free_distribution_id, free_license_sha256,
		 phase, status, error_code, active_dedup_key, retry_of_task_id, created_by, created_at, updated_at, started_at, completed_at)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`, task.ID, task.Version, task.Mode, task.GitHubRepository, task.WorkflowFile, task.SourceRef, task.SourceCommitSHA, task.FreeDistributionID, task.FreeLicenseSHA256,
		task.Phase, task.Status, task.ErrorCode, dedupKey, task.RetryOfTaskID, operatorID, task.CreatedAt, task.UpdatedAt,
		task.StartedAt, task.CompletedAt)
	if err != nil {
		var mysqlError *mysqldriver.MySQLError
		if errors.As(err, &mysqlError) && mysqlError.Number == 1062 && strings.Contains(mysqlError.Message, "uq_release_tasks_active_dedup") {
			return application.ErrReleaseDuplicate
		}
		return err
	}
	payload, _ := json.Marshal(map[string]any{"version": task.Version, "source_ref": task.SourceRef, "source_commit_sha": task.SourceCommitSHA, "free_distribution_id": task.FreeDistributionID, "free_license_sha256": task.FreeLicenseSHA256, "mode": task.Mode, "retry_of_task_id": task.RetryOfTaskID})
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events
		(id, operator_id, action, resource_type, resource_id, payload_json, created_at)
		VALUES (?, ?, 'release.task_created', 'release_task', ?, ?, ?)`, task.ID+"_created", operatorID, task.ID, payload, task.CreatedAt); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) RecordReleaseDispatch(ctx context.Context, taskID string, run domain.ReleaseRun) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	result, err := tx.ExecContext(ctx, `UPDATE release_tasks SET status='queued', phase='github_actions', updated_at=?,
		started_at=COALESCE(started_at, ?) WHERE id=? AND status='dispatching'`, run.CreatedAt, run.CreatedAt, taskID)
	if err != nil {
		return err
	}
	if changed, changeErr := result.RowsAffected(); changeErr != nil || changed != 1 {
		return fmt.Errorf("%w: release task is not dispatchable", application.ErrValidation)
	}
	if _, err = tx.ExecContext(ctx, `INSERT INTO release_runs
		(id, release_task_id, attempt, github_run_id, github_run_number, workflow_name, head_branch, head_sha,
		 status, conclusion, html_url, started_at, completed_at, synced_at, created_at)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`, run.ID, taskID, run.Attempt, run.GitHubRunID,
		run.GitHubRunNumber, run.WorkflowName, run.HeadBranch, run.HeadSHA, run.Status, run.Conclusion, run.HTMLURL,
		run.StartedAt, run.CompletedAt, run.SyncedAt, run.CreatedAt); err != nil {
		return err
	}
	payload, _ := json.Marshal(map[string]any{"github_run_id": run.GitHubRunID, "html_url": run.HTMLURL})
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events
		(id, operator_id, action, resource_type, resource_id, payload_json, created_at)
		SELECT ?, created_by, 'release.dispatched', 'release_task', id, ?, ? FROM release_tasks WHERE id=?`,
		taskID+"_dispatched", payload, run.CreatedAt, taskID); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) MarkReleaseTaskFailed(ctx context.Context, taskID, errorCode string) error {
	now := time.Now().UTC()
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, `UPDATE release_tasks SET status='failed', phase='failed', error_code=?,
		active_dedup_key=NULL, updated_at=?, completed_at=? WHERE id=?`, errorCode, now, now, taskID); err != nil {
		return err
	}
	payload, _ := json.Marshal(map[string]any{"error_code": errorCode})
	if _, err = tx.ExecContext(ctx, `INSERT IGNORE INTO audit_events
		(id, operator_id, action, resource_type, resource_id, payload_json, created_at)
		SELECT ?, created_by, 'release.failed', 'release_task', id, ?, ? FROM release_tasks WHERE id=?`,
		taskID+"_failed", payload, now, taskID); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) ApplyReleaseSnapshot(ctx context.Context, taskID string, snapshot ports.ReleaseSnapshot, status, phase string, errorCode *string, now time.Time) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	run := snapshot.Run
	if _, err = tx.ExecContext(ctx, `INSERT INTO release_runs
		(id, release_task_id, attempt, github_run_id, github_run_number, workflow_name, head_branch, head_sha,
		 status, conclusion, html_url, started_at, completed_at, synced_at, created_at)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
		ON DUPLICATE KEY UPDATE github_run_number=VALUES(github_run_number), workflow_name=VALUES(workflow_name),
		head_branch=VALUES(head_branch), head_sha=VALUES(head_sha), status=VALUES(status), conclusion=VALUES(conclusion),
		html_url=VALUES(html_url), started_at=VALUES(started_at), completed_at=VALUES(completed_at), synced_at=VALUES(synced_at)`,
		run.ID, taskID, run.Attempt, run.GitHubRunID, run.GitHubRunNumber, run.WorkflowName, run.HeadBranch, run.HeadSHA,
		run.Status, run.Conclusion, run.HTMLURL, run.StartedAt, run.CompletedAt, run.SyncedAt, run.CreatedAt); err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, `DELETE FROM release_run_jobs WHERE release_run_id=?`, run.ID); err != nil {
		return err
	}
	for _, job := range run.Jobs {
		if _, err = tx.ExecContext(ctx, `INSERT INTO release_run_jobs
			(id, release_run_id, github_job_id, sequence_no, name, runner_name, status, conclusion, html_url, started_at, completed_at)
			VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`, job.ID, run.ID, job.GitHubJobID, job.SequenceNo, job.Name,
			job.RunnerName, job.Status, job.Conclusion, job.HTMLURL, job.StartedAt, job.CompletedAt); err != nil {
			return err
		}
		for _, step := range job.Steps {
			if _, err = tx.ExecContext(ctx, `INSERT INTO release_run_steps
				(id, release_run_job_id, github_step_number, sequence_no, name, status, conclusion, failure_summary,
				 log_ref, started_at, completed_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`, step.ID, job.ID,
				step.GitHubStepNumber, step.SequenceNo, step.Name, step.Status, step.Conclusion, step.FailureSummary,
				step.LogRef, step.StartedAt, step.CompletedAt); err != nil {
				return err
			}
		}
	}
	for _, artifact := range snapshot.Artifacts {
		if _, err = tx.ExecContext(ctx, `INSERT INTO release_task_artifacts
			(id, release_task_id, release_run_id, github_artifact_id, name, file_name, platform, architecture, size_bytes, expires_at,
			 download_ref, github_digest_sha256, sha256, release_manifest_sha256, signature_key_id, runtime_linkage, verification_status,
			 verification_error_code, release_artifact_id, verified_at, created_at)
			VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
			ON DUPLICATE KEY UPDATE name=VALUES(name), file_name=VALUES(file_name), platform=VALUES(platform), architecture=VALUES(architecture), size_bytes=VALUES(size_bytes),
			expires_at=VALUES(expires_at), download_ref=VALUES(download_ref),
			github_digest_sha256=COALESCE(github_digest_sha256, VALUES(github_digest_sha256)),
			verification_status=IF(verification_status IN ('pending','unavailable'), VALUES(verification_status), verification_status)`,
			artifact.ID, taskID, run.ID, artifact.GitHubArtifactID, artifact.Name, artifact.FileName, artifact.Platform, artifact.Architecture, artifact.SizeBytes,
			artifact.ExpiresAt, artifact.DownloadRef, artifact.GitHubDigestSHA256, artifact.SHA256, artifact.ReleaseManifestSHA256, artifact.SignatureKeyID, artifact.RuntimeLinkage,
			artifact.VerificationStatus, artifact.VerificationErrorCode, artifact.ReleaseArtifactID, artifact.VerifiedAt,
			artifact.CreatedAt); err != nil {
			return err
		}
	}
	terminal := status == "failed" || status == "cancelled" || status == "completed"
	var completedAt any
	if terminal {
		completedAt = now
	}
	if _, err = tx.ExecContext(ctx, `UPDATE release_tasks SET status=?, phase=?, error_code=?,
		active_dedup_key=IF(?, NULL, active_dedup_key), updated_at=?, started_at=COALESCE(started_at, ?),
		completed_at=? WHERE id=? AND status NOT IN ('completed','failed','cancelled')`, status, phase, errorCode,
		terminal, now, run.StartedAt, completedAt, taskID); err != nil {
		return err
	}
	payload, _ := json.Marshal(map[string]any{"status": status, "phase": phase, "error_code": errorCode, "github_run_id": run.GitHubRunID})
	action := "release.status." + status
	if _, err = tx.ExecContext(ctx, `INSERT IGNORE INTO audit_events
		(id, operator_id, action, resource_type, resource_id, payload_json, created_at)
		SELECT ?, created_by, ?, 'release_task', id, ?, ? FROM release_tasks WHERE id=?`,
		taskID+"_status_"+status, action, payload, now, taskID); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) CompleteReleaseTaskArtifact(ctx context.Context, taskID, artifactID string, release domain.ReleaseArtifact, signatureKeyID string, now time.Time) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	var status, operatorID string
	if err = tx.QueryRowContext(ctx, `SELECT status, created_by FROM release_tasks WHERE id=? FOR UPDATE`, taskID).Scan(&status, &operatorID); err != nil {
		return err
	}
	var artifactStatus string
	if err = tx.QueryRowContext(ctx, `SELECT verification_status FROM release_task_artifacts WHERE id=? AND release_task_id=? FOR UPDATE`, artifactID, taskID).Scan(&artifactStatus); err != nil {
		return err
	}
	if artifactStatus == "verified" {
		return tx.Commit()
	}
	if status != "verifying" || artifactStatus != "queued" {
		return fmt.Errorf("%w: release task is not awaiting artifact verification", application.ErrValidation)
	}
	var existingID string
	err = tx.QueryRowContext(ctx, `SELECT id FROM release_artifacts WHERE version=? AND platform=? AND architecture=? AND sha256=?`,
		release.Version, release.Platform, release.Architecture, release.SHA256).Scan(&existingID)
	if errors.Is(err, sql.ErrNoRows) {
		existingID = release.ID
		if _, err = tx.ExecContext(ctx, `INSERT INTO release_artifacts
			(id, version, platform, architecture, object_key, sha256, release_manifest_sha256, size_bytes, signature_ref,
			 source_commit_sha, github_run_id, runtime_linkage, imported_by, created_at)
			VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`, release.ID, release.Version, release.Platform, release.Architecture,
			release.ObjectKey, release.SHA256, release.ReleaseManifestSHA256, release.SizeBytes, release.SignatureRef,
			release.SourceCommitSHA, release.GitHubRunID, release.RuntimeLinkage, operatorID, release.CreatedAt); err != nil {
			return err
		}
	} else if err != nil {
		return err
	}
	result, err := tx.ExecContext(ctx, `UPDATE release_task_artifacts SET sha256=?, release_manifest_sha256=?,
		signature_key_id=?, runtime_linkage=?, verification_status='verified', verification_error_code=NULL, release_artifact_id=?, verified_at=?
		WHERE id=? AND release_task_id=? AND verification_status<>'verified'`, release.SHA256, release.ReleaseManifestSHA256,
		signatureKeyID, release.RuntimeLinkage, existingID, now, artifactID, taskID)
	if err != nil {
		return err
	}
	if changed, rowsErr := result.RowsAffected(); rowsErr != nil || changed != 1 {
		return fmt.Errorf("%w: release task artifact is missing or already finalized", application.ErrValidation)
	}
	payload, _ := json.Marshal(map[string]any{"artifact_id": artifactID, "platform": release.Platform, "architecture": release.Architecture, "release_artifact_id": existingID, "sha256": release.SHA256,
		"release_manifest_sha256": release.ReleaseManifestSHA256, "signature_key_id": signatureKeyID})
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events
		(id, operator_id, action, resource_type, resource_id, payload_json, created_at)
		VALUES (?, ?, 'release.artifact_verified', 'release_task', ?, ?, ?)`, operationAuditID(artifactID, "artifact_verified"), operatorID, taskID, payload, now); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) FailReleaseTaskArtifact(ctx context.Context, taskID, artifactID, errorCode string, now time.Time) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if artifactID != "" {
		if _, err = tx.ExecContext(ctx, `UPDATE release_task_artifacts SET verification_status='failed',
			verification_error_code=? WHERE id=? AND release_task_id=? AND verification_status='queued'`, errorCode, artifactID, taskID); err != nil {
			return err
		}
	}
	payload, _ := json.Marshal(map[string]any{"error_code": errorCode, "artifact_id": artifactID})
	auditID := operationAuditID(artifactID+":"+now.Format(time.RFC3339Nano), "artifact_rejected")
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events
		(id, operator_id, action, resource_type, resource_id, payload_json, created_at)
		SELECT ?, created_by, 'release.artifact_rejected', 'release_task', id, ?, ? FROM release_tasks WHERE id=?`,
		auditID, payload, now, taskID); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) PrepareReleaseTaskArtifactVerification(ctx context.Context, taskID, artifactID, operatorID string, now time.Time) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	var taskStatus, artifactStatus string
	if err = tx.QueryRowContext(ctx, `SELECT status FROM release_tasks WHERE id=? FOR UPDATE`, taskID).Scan(&taskStatus); err != nil {
		return err
	}
	if taskStatus != "failed" && taskStatus != "completed" && taskStatus != "verifying" {
		return fmt.Errorf("%w: release task is not eligible for artifact reverification", application.ErrValidation)
	}
	if err = tx.QueryRowContext(ctx, `SELECT verification_status FROM release_task_artifacts WHERE id=? AND release_task_id=? FOR UPDATE`, artifactID, taskID).Scan(&artifactStatus); err != nil {
		return err
	}
	if artifactStatus == "verified" || artifactStatus == "queued" {
		return fmt.Errorf("%w: verified release artifacts cannot be reopened", application.ErrValidation)
	}
	result, err := tx.ExecContext(ctx, `UPDATE release_tasks SET status='verifying', phase='artifact_verification',
		error_code=NULL, active_dedup_key=CONCAT(version, ':', source_commit_sha), updated_at=?, completed_at=NULL
		WHERE id=? AND status IN ('failed','completed','verifying')`, now, taskID)
	if err != nil {
		var mysqlError *mysqldriver.MySQLError
		if errors.As(err, &mysqlError) && mysqlError.Number == 1062 {
			return application.ErrReleaseDuplicate
		}
		return err
	}
	if changed, rowsErr := result.RowsAffected(); rowsErr != nil || changed != 1 {
		return fmt.Errorf("%w: release task verification state changed concurrently", application.ErrValidation)
	}
	if _, err = tx.ExecContext(ctx, `UPDATE release_task_artifacts SET verification_status='queued',
		verification_error_code=NULL, sha256=NULL, release_manifest_sha256=NULL, signature_key_id='', runtime_linkage='',
		release_artifact_id=NULL, verified_at=NULL WHERE id=? AND release_task_id=? AND verification_status<>'verified'`, artifactID, taskID); err != nil {
		return err
	}
	payload, _ := json.Marshal(map[string]any{"artifact_id": artifactID})
	auditID := operationAuditID(taskID+":"+now.Format(time.RFC3339Nano), "artifact_reverification_requested")
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events
		(id, operator_id, action, resource_type, resource_id, payload_json, created_at)
		VALUES (?, ?, 'release.artifact_reverification_requested', 'release_task', ?, ?, ?)`, auditID, operatorID, taskID, payload, now); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) ListReleasePublishRequests(ctx context.Context, limit int) ([]domain.ReleasePublishRequest, error) {
	rows, err := store.db.QueryContext(ctx, `SELECT id, release_artifact_id, version, source_commit_sha, tag_name, status,
		error_code, requested_by, approved_by, executed_by, approval_comment, github_release_id, html_url,
		created_at, updated_at, approved_at, published_at
		FROM release_publish_requests ORDER BY created_at DESC, id DESC LIMIT ?`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := make([]domain.ReleasePublishRequest, 0, limit)
	for rows.Next() {
		item, err := scanReleasePublishRequest(rows)
		if err != nil {
			return nil, err
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func (store *Store) GetReleasePublishRequest(ctx context.Context, requestID string) (domain.ReleasePublishRequest, error) {
	item, err := scanReleasePublishRequest(store.db.QueryRowContext(ctx, `SELECT id, release_artifact_id, version,
		source_commit_sha, tag_name, status, error_code, requested_by, approved_by, executed_by, approval_comment,
		github_release_id, html_url, created_at, updated_at, approved_at, published_at
		FROM release_publish_requests WHERE id=?`, requestID))
	if errors.Is(err, sql.ErrNoRows) {
		return item, fmt.Errorf("%w: release publish request does not exist", application.ErrNotFound)
	}
	return item, err
}

func (store *Store) CreateReleasePublishRequest(ctx context.Context, request domain.ReleasePublishRequest) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	_, err = tx.ExecContext(ctx, `INSERT INTO release_publish_requests
		(id, release_artifact_id, version, source_commit_sha, tag_name, status, error_code, active_dedup_key,
		 requested_by, approved_by, executed_by, approval_comment, github_release_id, html_url, created_at, updated_at,
		 approved_at, published_at)
		VALUES (?, ?, ?, ?, ?, 'requested', NULL, ?, ?, NULL, NULL, '', NULL, '', ?, ?, NULL, NULL)`, request.ID,
		request.ReleaseArtifactID, request.Version, request.SourceCommitSHA, request.TagName, request.TagName,
		request.RequestedBy, request.CreatedAt, request.UpdatedAt)
	if err != nil {
		var mysqlError *mysqldriver.MySQLError
		if errors.As(err, &mysqlError) && mysqlError.Number == 1062 {
			return application.ErrReleasePublishDuplicate
		}
		return err
	}
	payload, _ := json.Marshal(map[string]any{"release_artifact_id": request.ReleaseArtifactID, "version": request.Version,
		"source_commit_sha": request.SourceCommitSHA, "tag_name": request.TagName})
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events
		(id, operator_id, action, resource_type, resource_id, payload_json, created_at)
		VALUES (?, ?, 'release.publish_requested', 'release_publish_request', ?, ?, ?)`, request.ID+"_requested",
		request.RequestedBy, request.ID, payload, request.CreatedAt); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) DecideReleasePublishRequest(ctx context.Context, requestID, decision, comment, operatorID string, now time.Time) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	var status, requestedBy string
	if err = tx.QueryRowContext(ctx, `SELECT status, requested_by FROM release_publish_requests WHERE id=? FOR UPDATE`, requestID).Scan(&status, &requestedBy); err != nil {
		return err
	}
	if status != "requested" || requestedBy == operatorID {
		return fmt.Errorf("%w: publish request is not independently approvable", application.ErrValidation)
	}
	approvalID := requestID + "_approval"
	if _, err = tx.ExecContext(ctx, `INSERT INTO release_publish_approvals
		(id, publish_request_id, decision, comment, decided_by, created_at) VALUES (?, ?, ?, ?, ?, ?)`,
		approvalID, requestID, decision, comment, operatorID, now); err != nil {
		return err
	}
	activeExpression := "active_dedup_key"
	if decision == "rejected" {
		activeExpression = "NULL"
	}
	query := `UPDATE release_publish_requests SET status=?, approved_by=?, approval_comment=?, approved_at=?,
		updated_at=?, active_dedup_key=` + activeExpression + ` WHERE id=? AND status='requested'`
	if _, err = tx.ExecContext(ctx, query, decision, operatorID, comment, now, now, requestID); err != nil {
		return err
	}
	payload, _ := json.Marshal(map[string]any{"decision": decision, "comment": comment})
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events
		(id, operator_id, action, resource_type, resource_id, payload_json, created_at)
		VALUES (?, ?, ?, 'release_publish_request', ?, ?, ?)`, approvalID+"_audit", operatorID,
		"release.publish_"+decision, requestID, payload, now); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) BeginReleasePublish(ctx context.Context, requestID, operatorID string, now time.Time) (domain.ReleasePublishRequest, error) {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelSerializable})
	if err != nil {
		return domain.ReleasePublishRequest{}, err
	}
	defer tx.Rollback()
	request, err := scanReleasePublishRequest(tx.QueryRowContext(ctx, `SELECT id, release_artifact_id, version,
		source_commit_sha, tag_name, status, error_code, requested_by, approved_by, executed_by, approval_comment,
		github_release_id, html_url, created_at, updated_at, approved_at, published_at
		FROM release_publish_requests WHERE id=? FOR UPDATE`, requestID))
	if err != nil {
		return domain.ReleasePublishRequest{}, err
	}
	if (request.Status != "approved" && request.Status != "failed") || request.ApprovedBy == nil || *request.ApprovedBy == request.RequestedBy {
		return domain.ReleasePublishRequest{}, fmt.Errorf("%w: publish request is not approved for execution", application.ErrValidation)
	}
	result, err := tx.ExecContext(ctx, `UPDATE release_publish_requests SET status='publishing', error_code=NULL,
		executed_by=?, updated_at=? WHERE id=? AND status IN ('approved','failed')`, operatorID, now, requestID)
	if err != nil {
		return domain.ReleasePublishRequest{}, err
	}
	if changed, rowsErr := result.RowsAffected(); rowsErr != nil || changed != 1 {
		return domain.ReleasePublishRequest{}, fmt.Errorf("%w: publish request execution is already in progress", application.ErrValidation)
	}
	payload, _ := json.Marshal(map[string]any{"tag_name": request.TagName, "release_artifact_id": request.ReleaseArtifactID})
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events
		(id, operator_id, action, resource_type, resource_id, payload_json, created_at)
		VALUES (?, ?, 'release.publish_started', 'release_publish_request', ?, ?, ?)`, operationAuditID(requestID, "publish_started_"+now.Format(time.RFC3339Nano)),
		operatorID, requestID, payload, now); err != nil {
		return domain.ReleasePublishRequest{}, err
	}
	if err = tx.Commit(); err != nil {
		return domain.ReleasePublishRequest{}, err
	}
	request.Status, request.ErrorCode, request.ExecutedBy, request.UpdatedAt = "publishing", nil, &operatorID, now
	return request, nil
}

func (store *Store) CompleteReleasePublish(ctx context.Context, requestID string, result ports.ReleasePublishResult, now time.Time) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	update, err := tx.ExecContext(ctx, `UPDATE release_publish_requests SET status='published', error_code=NULL,
		github_release_id=?, html_url=?, updated_at=?, published_at=? WHERE id=? AND status='publishing' AND tag_name=?`,
		result.GitHubReleaseID, result.HTMLURL, now, now, requestID, result.TagName)
	if err != nil {
		return err
	}
	if changed, rowsErr := update.RowsAffected(); rowsErr != nil || changed != 1 {
		return fmt.Errorf("%w: publish request is not finalizable", application.ErrValidation)
	}
	payload, _ := json.Marshal(result)
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events
		(id, operator_id, action, resource_type, resource_id, payload_json, created_at)
		SELECT ?, executed_by, 'release.published', 'release_publish_request', id, ?, ?
		FROM release_publish_requests WHERE id=?`, requestID+"_published", payload, now, requestID); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) FailReleasePublish(ctx context.Context, requestID, errorCode string, now time.Time) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, `UPDATE release_publish_requests SET status='failed', error_code=?, updated_at=?
		WHERE id=? AND status='publishing'`, errorCode, now, requestID); err != nil {
		return err
	}
	payload, _ := json.Marshal(map[string]string{"error_code": errorCode})
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events
		(id, operator_id, action, resource_type, resource_id, payload_json, created_at)
		SELECT ?, executed_by, 'release.publish_failed', 'release_publish_request', id, ?, ?
		FROM release_publish_requests WHERE id=?`, operationAuditID(requestID, "publish_failed_"+now.Format(time.RFC3339Nano)), payload, now, requestID); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) ListPublishingReleaseRequestIDs(ctx context.Context, limit int) ([]string, error) {
	rows, err := store.db.QueryContext(ctx, `SELECT id FROM release_publish_requests WHERE status='publishing'
		ORDER BY updated_at, id LIMIT ?`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	ids := make([]string, 0, limit)
	for rows.Next() {
		var id string
		if err := rows.Scan(&id); err != nil {
			return nil, err
		}
		ids = append(ids, id)
	}
	return ids, rows.Err()
}

func scanReleasePublishRequest(scanner rowScanner) (domain.ReleasePublishRequest, error) {
	var item domain.ReleasePublishRequest
	var errorCode, approvedBy, executedBy sql.NullString
	var githubReleaseID sql.NullInt64
	var approvedAt, publishedAt sql.NullTime
	err := scanner.Scan(&item.ID, &item.ReleaseArtifactID, &item.Version, &item.SourceCommitSHA, &item.TagName,
		&item.Status, &errorCode, &item.RequestedBy, &approvedBy, &executedBy, &item.ApprovalComment,
		&githubReleaseID, &item.HTMLURL, &item.CreatedAt, &item.UpdatedAt, &approvedAt, &publishedAt)
	if errorCode.Valid {
		item.ErrorCode = &errorCode.String
	}
	if approvedBy.Valid {
		item.ApprovedBy = &approvedBy.String
	}
	if executedBy.Valid {
		item.ExecutedBy = &executedBy.String
	}
	if githubReleaseID.Valid {
		item.GitHubReleaseID = &githubReleaseID.Int64
	}
	if approvedAt.Valid {
		item.ApprovedAt = &approvedAt.Time
	}
	if publishedAt.Valid {
		item.PublishedAt = &publishedAt.Time
	}
	return item, err
}

func (store *Store) ListPendingReleaseTaskIDs(ctx context.Context, limit int) ([]string, error) {
	rows, err := store.db.QueryContext(ctx, `SELECT id FROM release_tasks
		WHERE status IN ('dispatching', 'queued', 'in_progress', 'verifying') ORDER BY updated_at, id LIMIT ?`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	ids := make([]string, 0, limit)
	for rows.Next() {
		var id string
		if err := rows.Scan(&id); err != nil {
			return nil, err
		}
		ids = append(ids, id)
	}
	return ids, rows.Err()
}

func scanReleaseTask(scanner rowScanner) (domain.ReleaseTask, error) {
	var item domain.ReleaseTask
	var freeDistributionID, freeLicenseSHA256, errorCode, retryOf sql.NullString
	var startedAt, completedAt sql.NullTime
	err := scanner.Scan(&item.ID, &item.Version, &item.Mode, &item.GitHubRepository,
		&item.WorkflowFile, &item.SourceRef, &item.SourceCommitSHA, &freeDistributionID, &freeLicenseSHA256, &item.Phase, &item.Status, &errorCode, &retryOf,
		&item.CreatedBy, &item.CreatedAt, &item.UpdatedAt, &startedAt, &completedAt)
	if err != nil {
		return item, err
	}
	if errorCode.Valid {
		item.ErrorCode = &errorCode.String
	}
	if freeDistributionID.Valid {
		item.FreeDistributionID = freeDistributionID.String
	}
	if freeLicenseSHA256.Valid {
		item.FreeLicenseSHA256 = freeLicenseSHA256.String
	}
	if retryOf.Valid {
		item.RetryOfTaskID = &retryOf.String
	}
	if startedAt.Valid {
		item.StartedAt = &startedAt.Time
	}
	if completedAt.Valid {
		item.CompletedAt = &completedAt.Time
	}
	return item, nil
}

func loadReleaseRuns(ctx context.Context, tx *sql.Tx, taskID string, detail *domain.ReleaseTaskDetail) error {
	runRows, err := tx.QueryContext(ctx, `SELECT id, release_task_id, attempt, github_run_id, github_run_number,
		workflow_name, head_branch, head_sha, status, conclusion, html_url, started_at, completed_at, synced_at, created_at
		FROM release_runs WHERE release_task_id = ? ORDER BY attempt DESC`, taskID)
	if err != nil {
		return err
	}
	for runRows.Next() {
		var item domain.ReleaseRun
		var githubRunID, githubRunNumber sql.NullInt64
		var conclusion sql.NullString
		var startedAt, completedAt, syncedAt sql.NullTime
		if err = runRows.Scan(&item.ID, &item.ReleaseTaskID, &item.Attempt, &githubRunID, &githubRunNumber,
			&item.WorkflowName, &item.HeadBranch, &item.HeadSHA, &item.Status, &conclusion, &item.HTMLURL,
			&startedAt, &completedAt, &syncedAt, &item.CreatedAt); err != nil {
			runRows.Close()
			return err
		}
		if githubRunID.Valid {
			item.GitHubRunID = &githubRunID.Int64
		}
		if githubRunNumber.Valid {
			item.GitHubRunNumber = &githubRunNumber.Int64
		}
		if conclusion.Valid {
			item.Conclusion = &conclusion.String
		}
		if startedAt.Valid {
			item.StartedAt = &startedAt.Time
		}
		if completedAt.Valid {
			item.CompletedAt = &completedAt.Time
		}
		if syncedAt.Valid {
			item.SyncedAt = &syncedAt.Time
		}
		item.Jobs = []domain.ReleaseJob{}
		detail.Runs = append(detail.Runs, item)
	}
	if err = runRows.Err(); err != nil {
		runRows.Close()
		return err
	}
	runRows.Close()

	runIndex := make(map[string]int, len(detail.Runs))
	for index := range detail.Runs {
		runIndex[detail.Runs[index].ID] = index
	}
	jobRows, err := tx.QueryContext(ctx, `SELECT j.id, j.release_run_id, j.github_job_id, j.sequence_no, j.name,
		j.runner_name, j.status, j.conclusion, j.html_url, j.started_at, j.completed_at
		FROM release_run_jobs j JOIN release_runs r ON r.id = j.release_run_id
		WHERE r.release_task_id = ? ORDER BY r.attempt DESC, j.sequence_no`, taskID)
	if err != nil {
		return err
	}
	jobLocation := make(map[string][2]int)
	for jobRows.Next() {
		var item domain.ReleaseJob
		var conclusion sql.NullString
		var startedAt, completedAt sql.NullTime
		if err = jobRows.Scan(&item.ID, &item.ReleaseRunID, &item.GitHubJobID, &item.SequenceNo, &item.Name,
			&item.RunnerName, &item.Status, &conclusion, &item.HTMLURL, &startedAt, &completedAt); err != nil {
			jobRows.Close()
			return err
		}
		if conclusion.Valid {
			item.Conclusion = &conclusion.String
		}
		if startedAt.Valid {
			item.StartedAt = &startedAt.Time
		}
		if completedAt.Valid {
			item.CompletedAt = &completedAt.Time
		}
		item.Steps = []domain.ReleaseStep{}
		runPosition, ok := runIndex[item.ReleaseRunID]
		if !ok {
			jobRows.Close()
			return errors.New("release job refers to an unknown run")
		}
		detail.Runs[runPosition].Jobs = append(detail.Runs[runPosition].Jobs, item)
		jobLocation[item.ID] = [2]int{runPosition, len(detail.Runs[runPosition].Jobs) - 1}
	}
	if err = jobRows.Err(); err != nil {
		jobRows.Close()
		return err
	}
	jobRows.Close()

	stepRows, err := tx.QueryContext(ctx, `SELECT s.id, s.release_run_job_id, s.github_step_number, s.sequence_no,
		s.name, s.status, s.conclusion, s.failure_summary, s.log_ref, s.started_at, s.completed_at
		FROM release_run_steps s JOIN release_run_jobs j ON j.id = s.release_run_job_id
		JOIN release_runs r ON r.id = j.release_run_id WHERE r.release_task_id = ?
		ORDER BY r.attempt DESC, j.sequence_no, s.sequence_no`, taskID)
	if err != nil {
		return err
	}
	defer stepRows.Close()
	for stepRows.Next() {
		var item domain.ReleaseStep
		var conclusion sql.NullString
		var startedAt, completedAt sql.NullTime
		if err = stepRows.Scan(&item.ID, &item.ReleaseRunJobID, &item.GitHubStepNumber, &item.SequenceNo,
			&item.Name, &item.Status, &conclusion, &item.FailureSummary, &item.LogRef, &startedAt, &completedAt); err != nil {
			return err
		}
		if conclusion.Valid {
			item.Conclusion = &conclusion.String
		}
		if startedAt.Valid {
			item.StartedAt = &startedAt.Time
		}
		if completedAt.Valid {
			item.CompletedAt = &completedAt.Time
		}
		location, ok := jobLocation[item.ReleaseRunJobID]
		if !ok {
			return errors.New("release step refers to an unknown job")
		}
		runPosition, jobPosition := location[0], location[1]
		detail.Runs[runPosition].Jobs[jobPosition].Steps = append(detail.Runs[runPosition].Jobs[jobPosition].Steps, item)
	}
	return stepRows.Err()
}

func loadReleaseTaskArtifacts(ctx context.Context, tx queryContext, taskIDs []string) ([]domain.ReleaseTaskArtifact, error) {
	items := []domain.ReleaseTaskArtifact{}
	if len(taskIDs) == 0 {
		return items, nil
	}
	args := make([]any, len(taskIDs))
	for index, id := range taskIDs {
		args[index] = id
	}
	rows, err := tx.QueryContext(ctx, `SELECT id, release_task_id, release_run_id, github_artifact_id, name, file_name, platform, architecture,
		size_bytes, expires_at, download_ref, github_digest_sha256, sha256, release_manifest_sha256, signature_key_id, runtime_linkage,
		verification_status, verification_error_code, release_artifact_id, verified_at, created_at
		FROM release_task_artifacts WHERE release_task_id IN (`+strings.TrimSuffix(strings.Repeat("?,", len(taskIDs)), ",")+`) ORDER BY platform, created_at DESC, id DESC`, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	for rows.Next() {
		var item domain.ReleaseTaskArtifact
		var expiresAt, verifiedAt sql.NullTime
		var githubDigest, sha256Value, manifestSHA256, verificationError, releaseArtifactID sql.NullString
		if err = rows.Scan(&item.ID, &item.ReleaseTaskID, &item.ReleaseRunID, &item.GitHubArtifactID, &item.Name,
			&item.FileName, &item.Platform, &item.Architecture, &item.SizeBytes, &expiresAt, &item.DownloadRef, &githubDigest, &sha256Value, &manifestSHA256,
			&item.SignatureKeyID, &item.RuntimeLinkage, &item.VerificationStatus, &verificationError, &releaseArtifactID,
			&verifiedAt, &item.CreatedAt); err != nil {
			return nil, err
		}
		if expiresAt.Valid {
			item.ExpiresAt = &expiresAt.Time
		}
		if githubDigest.Valid {
			item.GitHubDigestSHA256 = &githubDigest.String
		}
		if sha256Value.Valid {
			item.SHA256 = &sha256Value.String
		}
		if manifestSHA256.Valid {
			item.ReleaseManifestSHA256 = &manifestSHA256.String
		}
		if verificationError.Valid {
			item.VerificationErrorCode = &verificationError.String
		}
		if releaseArtifactID.Valid {
			item.ReleaseArtifactID = &releaseArtifactID.String
		}
		if verifiedAt.Valid {
			item.VerifiedAt = &verifiedAt.Time
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func (store *Store) ValidateDelivery(ctx context.Context, input domain.DeliveryInput) error {
	var matched int
	err := store.db.QueryRowContext(ctx, `SELECT COUNT(*) FROM license_records l
		WHERE l.license_id = ? AND l.customer_id = ? AND l.status = 'active'
		  AND ((? IS NOT NULL AND l.source_type = 'order' AND l.source_id = ? AND EXISTS (
		    SELECT 1 FROM orders o WHERE o.id = ? AND o.customer_id = ? AND o.status = 'fulfilled'))
		   OR (? IS NOT NULL AND l.source_type = 'trial' AND l.source_id = ? AND EXISTS (
		    SELECT 1 FROM trials t WHERE t.id = ? AND t.customer_id = ? AND t.status IN ('active','converted'))))`,
		input.LicenseID, input.CustomerID,
		input.OrderID, input.OrderID, input.OrderID, input.CustomerID,
		input.TrialID, input.TrialID, input.TrialID, input.CustomerID).Scan(&matched)
	if err != nil {
		return err
	}
	if matched != 1 {
		return fmt.Errorf("%w: delivery source, customer and license do not match", application.ErrValidation)
	}
	return nil
}

func (store *Store) CreateDelivery(ctx context.Context, delivery domain.DeliveryRecord, operatorID string) error {
	tx, err := store.db.BeginTx(ctx, &sql.TxOptions{Isolation: sql.LevelReadCommitted})
	if err != nil {
		return err
	}
	defer tx.Rollback()
	result, err := tx.ExecContext(ctx, `INSERT INTO delivery_records
		(id, customer_id, order_id, trial_id, license_id, release_artifact_id, channel, recipient,
		 receipt_object_key, receipt_sha256, delivered_at, created_by, created_at)
		SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ? FROM license_records l
		WHERE l.license_id = ? AND l.customer_id = ? AND l.status = 'active'
		  AND ((? IS NOT NULL AND l.source_type = 'order' AND l.source_id = ? AND EXISTS (
		    SELECT 1 FROM orders o WHERE o.id = ? AND o.customer_id = ? AND o.status = 'fulfilled'))
		   OR (? IS NOT NULL AND l.source_type = 'trial' AND l.source_id = ? AND EXISTS (
		    SELECT 1 FROM trials t WHERE t.id = ? AND t.customer_id = ? AND t.status IN ('active','converted'))))`, delivery.ID, delivery.CustomerID, delivery.OrderID,
		delivery.TrialID, delivery.LicenseID, delivery.ReleaseArtifactID, delivery.Channel, delivery.Recipient,
		delivery.ReceiptObjectKey, delivery.ReceiptSHA256, delivery.DeliveredAt, operatorID, delivery.CreatedAt,
		delivery.LicenseID, delivery.CustomerID,
		delivery.OrderID, delivery.OrderID, delivery.OrderID, delivery.CustomerID,
		delivery.TrialID, delivery.TrialID, delivery.TrialID, delivery.CustomerID)
	if err != nil {
		return err
	}
	if changed, changeErr := result.RowsAffected(); changeErr != nil || changed != 1 {
		return fmt.Errorf("%w: delivery source, customer and license do not match", application.ErrValidation)
	}
	payload, _ := json.Marshal(delivery)
	if _, err = tx.ExecContext(ctx, `INSERT INTO audit_events
		(id, operator_id, action, resource_type, resource_id, payload_json, created_at)
		VALUES (?, ?, 'delivery.created', 'delivery', ?, ?, ?)`, delivery.ID+"_created", operatorID, delivery.ID, payload, delivery.CreatedAt); err != nil {
		return err
	}
	return tx.Commit()
}

func (store *Store) ListDeliveries(ctx context.Context, limit int) ([]domain.DeliveryRecord, error) {
	return listDeliveries(ctx, store.db, limit)
}

func listDeliveries(ctx context.Context, database queryContext, limit int) ([]domain.DeliveryRecord, error) {
	rows, err := database.QueryContext(ctx, `SELECT id, customer_id, order_id, trial_id, license_id, release_artifact_id,
		channel, recipient, receipt_object_key, receipt_sha256, delivered_at, created_at
		FROM delivery_records ORDER BY created_at DESC LIMIT ?`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	items := make([]domain.DeliveryRecord, 0, limit)
	for rows.Next() {
		var item domain.DeliveryRecord
		var orderID, trialID sql.NullString
		var deliveredAt sql.NullTime
		if err := rows.Scan(&item.ID, &item.CustomerID, &orderID, &trialID, &item.LicenseID, &item.ReleaseArtifactID,
			&item.Channel, &item.Recipient, &item.ReceiptObjectKey, &item.ReceiptSHA256, &deliveredAt, &item.CreatedAt); err != nil {
			return nil, err
		}
		if orderID.Valid {
			item.OrderID = &orderID.String
		}
		if trialID.Valid {
			item.TrialID = &trialID.String
		}
		if deliveredAt.Valid {
			item.DeliveredAt = &deliveredAt.Time
		}
		items = append(items, item)
	}
	return items, rows.Err()
}

func (store *Store) GetDelivery(ctx context.Context, deliveryID string) (domain.DeliveryRecord, error) {
	var item domain.DeliveryRecord
	var orderID, trialID sql.NullString
	var deliveredAt sql.NullTime
	err := store.db.QueryRowContext(ctx, `SELECT id, customer_id, order_id, trial_id, license_id, release_artifact_id,
		channel, recipient, receipt_object_key, receipt_sha256, delivered_at, created_at
		FROM delivery_records WHERE id = ?`, deliveryID).Scan(&item.ID, &item.CustomerID, &orderID, &trialID,
		&item.LicenseID, &item.ReleaseArtifactID, &item.Channel, &item.Recipient, &item.ReceiptObjectKey,
		&item.ReceiptSHA256, &deliveredAt, &item.CreatedAt)
	if errors.Is(err, sql.ErrNoRows) {
		return item, fmt.Errorf("%w: delivery does not exist", application.ErrNotFound)
	}
	if orderID.Valid {
		item.OrderID = &orderID.String
	}
	if trialID.Valid {
		item.TrialID = &trialID.String
	}
	if deliveredAt.Valid {
		item.DeliveredAt = &deliveredAt.Time
	}
	return item, err
}
