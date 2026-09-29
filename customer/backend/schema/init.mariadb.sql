CREATE TABLE schema_migrations (
  version INT UNSIGNED PRIMARY KEY,
  name VARCHAR(160) NOT NULL UNIQUE,
  checksum_sha256 CHAR(64) NOT NULL,
  applied_at CHAR(24) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE identities (
  id VARCHAR(128) PRIMARY KEY,
  email VARCHAR(320) NOT NULL,
  display_name VARCHAR(160) NOT NULL,
  password_hash VARCHAR(512) NOT NULL,
  role VARCHAR(16) NOT NULL,
  status VARCHAR(16) NOT NULL DEFAULT 'active',
  active_email VARCHAR(320) GENERATED ALWAYS AS (
    CASE WHEN status <> 'deleted' THEN email ELSE NULL END
  ) STORED,
  can_consume_model TINYINT UNSIGNED NOT NULL DEFAULT 0,
  password_change_required TINYINT UNSIGNED NOT NULL DEFAULT 1,
  revision BIGINT UNSIGNED NOT NULL DEFAULT 0,
  integrity_hmac VARCHAR(64) NOT NULL,
  created_at CHAR(24) NOT NULL,
  updated_at CHAR(24) NOT NULL,
  CONSTRAINT chk_identities_role CHECK (role IN ('owner', 'admin', 'member')),
  CONSTRAINT chk_identities_status CHECK (status IN ('active', 'disabled', 'deleted')),
  CONSTRAINT chk_identities_consume CHECK (can_consume_model IN (0, 1)),
  CONSTRAINT chk_identities_password_change CHECK (password_change_required IN (0, 1)),
  UNIQUE KEY identities_active_email_uq(active_email),
  INDEX identities_seat_idx(status, can_consume_model)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE transaction_gates (
  gate_key VARCHAR(64) PRIMARY KEY,
  revision BIGINT UNSIGNED NOT NULL DEFAULT 0
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;
INSERT INTO transaction_gates(gate_key, revision) VALUES ('member_seat', 0), ('audit_log', 0);

CREATE TABLE sessions (
  id VARCHAR(128) PRIMARY KEY,
  identity_id VARCHAR(128) NOT NULL,
  token_hash CHAR(64) NOT NULL UNIQUE,
  expires_at CHAR(24) NOT NULL,
  integrity_hmac VARCHAR(64) NOT NULL,
  created_at CHAR(24) NOT NULL,
  CONSTRAINT sessions_identity_fk FOREIGN KEY (identity_id) REFERENCES identities(id) ON DELETE CASCADE,
  INDEX sessions_token_idx(token_hash)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE api_keys (
  id VARCHAR(128) PRIMARY KEY,
  identity_id VARCHAR(128) NOT NULL,
  name VARCHAR(160) NOT NULL,
  key_hash CHAR(64) NOT NULL UNIQUE,
  key_prefix VARCHAR(32) NOT NULL,
  status VARCHAR(16) NOT NULL DEFAULT 'active',
  revision BIGINT UNSIGNED NOT NULL DEFAULT 0,
  integrity_hmac VARCHAR(64) NOT NULL,
  created_at CHAR(24) NOT NULL,
  last_used_at CHAR(24),
  CONSTRAINT api_keys_status_chk CHECK (status IN ('active', 'suspended', 'revoked')),
  CONSTRAINT api_keys_identity_fk FOREIGN KEY (identity_id) REFERENCES identities(id) ON DELETE CASCADE,
  INDEX api_keys_hash_idx(key_hash)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE user_balances (
  identity_id VARCHAR(128) PRIMARY KEY,
  balance_tokens BIGINT NOT NULL DEFAULT 0,
  reserved_tokens BIGINT UNSIGNED NOT NULL DEFAULT 0,
  granted_tokens BIGINT UNSIGNED NOT NULL DEFAULT 0,
  consumed_tokens BIGINT UNSIGNED NOT NULL DEFAULT 0,
  request_count BIGINT UNSIGNED NOT NULL DEFAULT 0,
  raw_tokens BIGINT UNSIGNED NOT NULL DEFAULT 0,
  billed_tokens BIGINT UNSIGNED NOT NULL DEFAULT 0,
  uncached_input BIGINT UNSIGNED NOT NULL DEFAULT 0,
  cached_input BIGINT UNSIGNED NOT NULL DEFAULT 0,
  cache_write BIGINT UNSIGNED NOT NULL DEFAULT 0,
  output_tokens BIGINT UNSIGNED NOT NULL DEFAULT 0,
  revision BIGINT UNSIGNED NOT NULL DEFAULT 0,
  last_ledger_hmac VARCHAR(64) NOT NULL DEFAULT '',
  integrity_hmac VARCHAR(64) NOT NULL,
  updated_at CHAR(24) NOT NULL,
  CONSTRAINT user_balances_identity_fk FOREIGN KEY (identity_id) REFERENCES identities(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE ledger_entries (
  id VARCHAR(128) PRIMARY KEY,
  identity_id VARCHAR(128) NOT NULL,
  kind VARCHAR(32) NOT NULL,
  amount_tokens BIGINT NOT NULL,
  uncached_input BIGINT UNSIGNED NOT NULL DEFAULT 0,
  cached_input BIGINT UNSIGNED NOT NULL DEFAULT 0,
  cache_write BIGINT UNSIGNED NOT NULL DEFAULT 0,
  output_tokens BIGINT UNSIGNED NOT NULL DEFAULT 0,
  uncovered_tokens BIGINT UNSIGNED NOT NULL DEFAULT 0,
  raw_tokens BIGINT UNSIGNED NOT NULL DEFAULT 0,
  billed_tokens BIGINT UNSIGNED NOT NULL DEFAULT 0,
  multiplier_micros BIGINT UNSIGNED NOT NULL DEFAULT 1000000,
  reference_id VARCHAR(160),
  description VARCHAR(512) NOT NULL,
  protocol VARCHAR(32) NOT NULL DEFAULT '',
  model VARCHAR(160) NOT NULL DEFAULT '',
  api_key_id VARCHAR(128),
  runner_id VARCHAR(128),
  previous_entry_hmac VARCHAR(64) NOT NULL,
  integrity_hmac VARCHAR(64) NOT NULL,
  created_at CHAR(24) NOT NULL,
  CONSTRAINT ledger_identity_fk FOREIGN KEY (identity_id) REFERENCES identities(id),
  CONSTRAINT ledger_api_key_fk FOREIGN KEY (api_key_id) REFERENCES api_keys(id) ON DELETE SET NULL,
  UNIQUE KEY ledger_identity_kind_reference_uq(identity_id, kind, reference_id),
  INDEX ledger_identity_time_idx(identity_id, created_at DESC)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE money_balances (
  identity_id VARCHAR(128) PRIMARY KEY,
  currency CHAR(3) NOT NULL,
  balance_nanos BIGINT NOT NULL,
  credited_nanos BIGINT NOT NULL,
  debited_nanos BIGINT NOT NULL,
  revision BIGINT NOT NULL,
  last_ledger_hmac VARCHAR(128) NOT NULL,
  integrity_hmac VARCHAR(128) NOT NULL,
  updated_at CHAR(24) NOT NULL,
  CONSTRAINT money_balances_currency_chk CHECK (currency IN ('CNY', 'USD')),
  CONSTRAINT money_balances_counters_chk CHECK (credited_nanos >= 0 AND debited_nanos >= 0 AND revision >= 0),
  CONSTRAINT money_balances_identity_fk FOREIGN KEY (identity_id) REFERENCES identities(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE money_ledger_entries (
  id VARCHAR(128) PRIMARY KEY,
  identity_id VARCHAR(128) NOT NULL,
  currency CHAR(3) NOT NULL,
  kind VARCHAR(16) NOT NULL,
  amount_nanos BIGINT NOT NULL,
  balance_revision BIGINT NOT NULL,
  reference_id VARCHAR(160) NOT NULL,
  billing_status VARCHAR(16) NOT NULL,
  details_json TEXT NOT NULL,
  previous_entry_hmac VARCHAR(128) NOT NULL,
  integrity_hmac VARCHAR(128) NOT NULL,
  created_at CHAR(24) NOT NULL,
  CONSTRAINT money_ledger_currency_chk CHECK (currency IN ('CNY', 'USD')),
  CONSTRAINT money_ledger_kind_chk CHECK (kind IN ('grant', 'charge', 'anomaly', 'request_failed', 'adjustment', 'refund')),
  CONSTRAINT money_ledger_status_chk CHECK (billing_status IN ('not_applicable', 'charged', 'billing_error')),
  CONSTRAINT money_ledger_revision_chk CHECK (balance_revision > 0),
  CONSTRAINT money_ledger_identity_fk FOREIGN KEY (identity_id) REFERENCES identities(id),
  UNIQUE KEY money_ledger_identity_reference_uq(identity_id, reference_id),
  UNIQUE KEY money_ledger_identity_revision_uq(identity_id, balance_revision),
  INDEX money_ledger_identity_time_idx(identity_id, created_at DESC)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE quota_reservations (
  id VARCHAR(128) PRIMARY KEY,
  identity_id VARCHAR(128) NOT NULL,
  api_key_id VARCHAR(128) NOT NULL,
  request_id VARCHAR(160) NOT NULL UNIQUE,
  reserved_tokens BIGINT UNSIGNED NOT NULL,
  status VARCHAR(16) NOT NULL DEFAULT 'active',
  revision BIGINT UNSIGNED NOT NULL DEFAULT 0,
  integrity_hmac VARCHAR(64) NOT NULL,
  created_at CHAR(24) NOT NULL,
  expires_at CHAR(24) NOT NULL,
  settled_at CHAR(24),
  CONSTRAINT quota_reservations_positive_chk CHECK (reserved_tokens > 0),
  CONSTRAINT quota_reservations_status_chk CHECK (status IN ('active', 'released', 'settled')),
  CONSTRAINT quota_reservations_identity_fk FOREIGN KEY (identity_id) REFERENCES identities(id),
  CONSTRAINT quota_reservations_api_key_fk FOREIGN KEY (api_key_id) REFERENCES api_keys(id),
  INDEX quota_reservations_identity_status_idx(identity_id, status, expires_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE vouchers (
  id VARCHAR(128) PRIMARY KEY,
  code_hash CHAR(64) NOT NULL UNIQUE,
  code_prefix VARCHAR(32) NOT NULL,
  name VARCHAR(160) NOT NULL,
  quota_tokens BIGINT UNSIGNED NOT NULL,
  status VARCHAR(16) NOT NULL DEFAULT 'active',
  max_redemptions INT UNSIGNED NOT NULL DEFAULT 1,
  redeemed_count INT UNSIGNED NOT NULL DEFAULT 0,
  expires_at CHAR(24),
  created_by VARCHAR(128) NOT NULL,
  revision BIGINT UNSIGNED NOT NULL DEFAULT 0,
  integrity_hmac VARCHAR(128) NOT NULL,
  created_at CHAR(24) NOT NULL,
  CONSTRAINT vouchers_status_chk CHECK (status IN ('active', 'disabled', 'redeemed')),
  CONSTRAINT vouchers_creator_fk FOREIGN KEY (created_by) REFERENCES identities(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE voucher_deliveries (
  id VARCHAR(128) PRIMARY KEY,
  voucher_id VARCHAR(128) NOT NULL,
  identity_id VARCHAR(128) NOT NULL,
  status VARCHAR(16) NOT NULL DEFAULT 'pending',
  delivered_at CHAR(24) NOT NULL,
  redeemed_at CHAR(24),
  CONSTRAINT voucher_deliveries_status_chk CHECK (status IN ('pending', 'redeemed')),
  CONSTRAINT voucher_deliveries_voucher_fk FOREIGN KEY (voucher_id) REFERENCES vouchers(id) ON DELETE CASCADE,
  CONSTRAINT voucher_deliveries_identity_fk FOREIGN KEY (identity_id) REFERENCES identities(id) ON DELETE CASCADE,
  UNIQUE KEY voucher_deliveries_voucher_identity_uq(voucher_id, identity_id),
  INDEX voucher_deliveries_identity_idx(identity_id, delivered_at DESC)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE voucher_redemptions (
  id VARCHAR(128) PRIMARY KEY,
  voucher_id VARCHAR(128) NOT NULL,
  identity_id VARCHAR(128) NOT NULL,
  amount_tokens BIGINT UNSIGNED NOT NULL,
  ledger_entry_id VARCHAR(128) NOT NULL UNIQUE,
  created_at CHAR(24) NOT NULL,
  CONSTRAINT voucher_redemptions_voucher_fk FOREIGN KEY (voucher_id) REFERENCES vouchers(id),
  CONSTRAINT voucher_redemptions_identity_fk FOREIGN KEY (identity_id) REFERENCES identities(id),
  CONSTRAINT voucher_redemptions_ledger_fk FOREIGN KEY (ledger_entry_id) REFERENCES ledger_entries(id),
  UNIQUE KEY voucher_redemptions_voucher_identity_uq(voucher_id, identity_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE quota_requests (
  id VARCHAR(128) PRIMARY KEY,
  identity_id VARCHAR(128) NOT NULL,
  amount_nanos BIGINT UNSIGNED NOT NULL,
  reason VARCHAR(512) NOT NULL,
  status VARCHAR(16) NOT NULL DEFAULT 'pending',
  pending_identity_id VARCHAR(128) GENERATED ALWAYS AS (CASE WHEN status = 'pending' THEN identity_id ELSE NULL END) STORED,
  review_note VARCHAR(512) NOT NULL DEFAULT '',
  reviewed_by VARCHAR(128),
  reviewed_at CHAR(24),
  revision BIGINT UNSIGNED NOT NULL DEFAULT 0,
  integrity_hmac VARCHAR(128) NOT NULL,
  created_at CHAR(24) NOT NULL,
  CONSTRAINT quota_requests_status_chk CHECK (status IN ('pending', 'approved', 'rejected')),
  CONSTRAINT quota_requests_identity_fk FOREIGN KEY (identity_id) REFERENCES identities(id),
  CONSTRAINT quota_requests_reviewer_fk FOREIGN KEY (reviewed_by) REFERENCES identities(id),
  UNIQUE KEY quota_requests_one_pending_uq(pending_identity_id),
  INDEX quota_requests_identity_idx(identity_id, created_at DESC),
  INDEX quota_requests_status_idx(status, created_at DESC)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE admin_quota_adjustments (
  id VARCHAR(128) PRIMARY KEY,
  identity_id VARCHAR(128) NOT NULL,
  admin_identity_id VARCHAR(128) NOT NULL,
  amount_tokens BIGINT UNSIGNED NOT NULL,
  reason VARCHAR(512) NOT NULL,
  ledger_entry_id VARCHAR(128) NOT NULL UNIQUE,
  created_at CHAR(24) NOT NULL,
  CONSTRAINT quota_adjustments_identity_fk FOREIGN KEY (identity_id) REFERENCES identities(id),
  CONSTRAINT quota_adjustments_admin_fk FOREIGN KEY (admin_identity_id) REFERENCES identities(id),
  CONSTRAINT quota_adjustments_ledger_fk FOREIGN KEY (ledger_entry_id) REFERENCES ledger_entries(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE runtime_settings (
  setting_key VARCHAR(160) PRIMARY KEY,
  setting_value TEXT NOT NULL,
  revision BIGINT UNSIGNED NOT NULL DEFAULT 0,
  integrity_hmac VARCHAR(128) NOT NULL,
  updated_at CHAR(24) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE runner_enrollments (
  id VARCHAR(128) PRIMARY KEY,
  token_hash CHAR(64) NOT NULL UNIQUE,
  token_prefix VARCHAR(32) NOT NULL,
  runner_name VARCHAR(160) NOT NULL,
  status VARCHAR(16) NOT NULL DEFAULT 'pending',
  expires_at CHAR(24) NOT NULL,
  used_at CHAR(24),
  runner_id VARCHAR(128),
  created_by VARCHAR(128) NOT NULL,
  created_at CHAR(24) NOT NULL,
  CONSTRAINT runner_enrollments_status_chk CHECK (status IN ('pending', 'used', 'expired', 'cancelled')),
  CONSTRAINT runner_enrollments_creator_fk FOREIGN KEY (created_by) REFERENCES identities(id),
  INDEX runner_enrollments_token_idx(token_hash)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE runners (
  id VARCHAR(128) PRIMARY KEY,
  enrollment_id VARCHAR(128) NOT NULL UNIQUE,
  name VARCHAR(160) NOT NULL,
  credential_hash CHAR(64) NOT NULL UNIQUE,
  enabled TINYINT UNSIGNED NOT NULL DEFAULT 1,
  version VARCHAR(64) NOT NULL,
  protocol_version INT UNSIGNED NOT NULL,
  platform VARCHAR(32) NOT NULL,
  architecture VARCHAR(32) NOT NULL,
  max_inflight INT UNSIGNED NOT NULL DEFAULT 1,
  inflight INT UNSIGNED NOT NULL DEFAULT 0,
  recent_request_count INT UNSIGNED NOT NULL DEFAULT 0,
  recent_error_count INT UNSIGNED NOT NULL DEFAULT 0,
  latency_ms INT UNSIGNED NOT NULL DEFAULT 0,
  last_seen_at CHAR(24),
  created_at CHAR(24) NOT NULL,
  updated_at CHAR(24) NOT NULL,
  CONSTRAINT runners_enabled_chk CHECK (enabled IN (0, 1)),
  CONSTRAINT runners_inflight_chk CHECK (inflight <= max_inflight),
  CONSTRAINT runners_enrollment_fk FOREIGN KEY (enrollment_id) REFERENCES runner_enrollments(id),
  INDEX runners_available_idx(enabled, protocol_version, last_seen_at DESC)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE upstream_accounts (
  id VARCHAR(128) PRIMARY KEY,
  provider VARCHAR(32) NOT NULL,
  subject_id VARCHAR(256) NOT NULL,
  email VARCHAR(320) NOT NULL,
  plan VARCHAR(64) NOT NULL DEFAULT '',
  status VARCHAR(16) NOT NULL DEFAULT 'active',
  last_success_runner_id VARCHAR(128),
  last_verified_at CHAR(24),
  created_at CHAR(24) NOT NULL,
  updated_at CHAR(24) NOT NULL,
  CONSTRAINT upstream_accounts_status_chk CHECK (status IN ('active', 'disabled', 'invalid')),
  CONSTRAINT upstream_accounts_runner_fk FOREIGN KEY (last_success_runner_id) REFERENCES runners(id) ON DELETE SET NULL,
  UNIQUE KEY upstream_accounts_provider_subject_uq(provider, subject_id),
  INDEX upstream_accounts_status_idx(provider, status, updated_at DESC)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE upstream_credential_instances (
  id VARCHAR(128) PRIMARY KEY,
  account_id VARCHAR(128) NOT NULL,
  credential_identity_hmac VARCHAR(64) NOT NULL,
  encrypted_payload LONGBLOB NOT NULL,
  payload_nonce VARBINARY(64) NOT NULL,
  wrapped_data_key VARBINARY(128) NOT NULL,
  wrap_nonce VARBINARY(64) NOT NULL,
  encryption_version INT UNSIGNED NOT NULL DEFAULT 1,
  credential_revision BIGINT UNSIGNED NOT NULL DEFAULT 0,
  expires_at CHAR(24) NOT NULL,
  status VARCHAR(16) NOT NULL DEFAULT 'active',
  last_refreshed_at CHAR(24),
  created_at CHAR(24) NOT NULL,
  updated_at CHAR(24) NOT NULL,
  CONSTRAINT credential_instances_encryption_chk CHECK (encryption_version = 1),
  CONSTRAINT credential_instances_status_chk CHECK (status IN ('active', 'refreshing', 'invalid', 'revoked')),
  CONSTRAINT credential_instances_account_fk FOREIGN KEY (account_id) REFERENCES upstream_accounts(id) ON DELETE CASCADE,
  UNIQUE KEY credential_instances_identity_uq(credential_identity_hmac),
  INDEX credential_instances_account_idx(account_id, status, expires_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE credential_refresh_leases (
  credential_instance_id VARCHAR(128) PRIMARY KEY,
  lease_token_hash CHAR(64) NOT NULL UNIQUE,
  expected_revision BIGINT UNSIGNED NOT NULL,
  expires_at CHAR(24) NOT NULL,
  created_at CHAR(24) NOT NULL,
  CONSTRAINT refresh_leases_credential_fk FOREIGN KEY (credential_instance_id) REFERENCES upstream_credential_instances(id) ON DELETE CASCADE,
  INDEX credential_refresh_leases_expiry_idx(expires_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE models (
  id VARCHAR(128) PRIMARY KEY,
  public_name VARCHAR(160) NOT NULL UNIQUE,
  display_name VARCHAR(160) NOT NULL DEFAULT '',
  enabled TINYINT UNSIGNED NOT NULL DEFAULT 1,
  discovered_at CHAR(24),
  created_at CHAR(24) NOT NULL,
  CONSTRAINT models_enabled_chk CHECK (enabled IN (0, 1))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE account_models (
  account_id VARCHAR(128) NOT NULL,
  model_id VARCHAR(128) NOT NULL,
  upstream_name VARCHAR(160) NOT NULL,
  discovered_at CHAR(24) NOT NULL,
  PRIMARY KEY (account_id, model_id),
  CONSTRAINT account_models_account_fk FOREIGN KEY (account_id) REFERENCES upstream_accounts(id) ON DELETE CASCADE,
  CONSTRAINT account_models_model_fk FOREIGN KEY (model_id) REFERENCES models(id) ON DELETE CASCADE,
  INDEX account_models_model_idx(model_id, discovered_at DESC)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE audit_events (
  id VARCHAR(128) PRIMARY KEY,
  event_sequence BIGINT UNSIGNED NOT NULL UNIQUE,
  actor_identity_id VARCHAR(128),
  actor_role VARCHAR(16) NOT NULL,
  action VARCHAR(128) NOT NULL,
  target_type VARCHAR(64) NOT NULL,
  target_id VARCHAR(128),
  outcome VARCHAR(16) NOT NULL,
  previous_event_hmac CHAR(43) NOT NULL,
  integrity_hmac CHAR(43) NOT NULL,
  created_at CHAR(24) NOT NULL,
  CONSTRAINT audit_events_sequence_chk CHECK (event_sequence > 0),
  CONSTRAINT audit_events_actor_role_chk CHECK (actor_role IN ('owner', 'admin', 'member', 'system', 'runner')),
  CONSTRAINT audit_events_outcome_chk CHECK (outcome IN ('succeeded', 'failed')),
  INDEX audit_events_time_idx(created_at DESC, event_sequence DESC),
  INDEX audit_events_actor_idx(actor_identity_id, event_sequence DESC),
  INDEX audit_events_action_idx(action, event_sequence DESC)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE security_state (
  state_key VARCHAR(160) PRIMARY KEY,
  state_value LONGBLOB NOT NULL,
  revision BIGINT UNSIGNED NOT NULL,
  mac VARBINARY(64) NOT NULL,
  updated_at CHAR(24) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

-- 2.2.0 fresh-install schema; previous releases are not upgrade inputs.

ALTER TABLE ledger_entries
  ADD COLUMN requested_model VARCHAR(160) NULL AFTER model,
  ADD COLUMN processing_tier VARCHAR(16) NULL AFTER requested_model,
  ADD COLUMN reasoning_effort VARCHAR(16) NULL AFTER processing_tier;

ALTER TABLE ledger_entries
  ADD COLUMN client_request_id VARCHAR(160) NULL AFTER reference_id,
  ADD INDEX ledger_identity_client_request_idx(identity_id, api_key_id, client_request_id);

INSERT INTO transaction_gates(gate_key, revision) VALUES ('entity_quota', 0)
ON DUPLICATE KEY UPDATE gate_key='entity_quota';

CREATE TABLE upstream_connections (
  id VARCHAR(128) PRIMARY KEY,
  provider VARCHAR(32) NOT NULL,
  channel_id VARCHAR(128) NOT NULL,
  endpoint_profile VARCHAR(128) NOT NULL,
  auth_scheme VARCHAR(16) NOT NULL,
  billing_mode VARCHAR(16) NOT NULL,
  display_name VARCHAR(160) NOT NULL,
  legacy_account_id VARCHAR(128),
  status VARCHAR(16) NOT NULL DEFAULT 'active',
  revision BIGINT NOT NULL DEFAULT 1,
  created_at CHAR(24) NOT NULL,
  updated_at CHAR(24) NOT NULL,
  CONSTRAINT upstream_connections_auth_chk CHECK (auth_scheme IN ('api_key', 'oauth')),
  CONSTRAINT upstream_connections_billing_chk CHECK (billing_mode IN ('usage', 'coding_plan', 'subscription')),
  CONSTRAINT upstream_connections_status_chk CHECK (status IN ('active', 'disabled', 'invalid')),
  CONSTRAINT upstream_connections_revision_chk CHECK (revision > 0),
  CONSTRAINT upstream_connections_legacy_fk FOREIGN KEY (legacy_account_id) REFERENCES upstream_accounts(id) ON DELETE CASCADE,
  UNIQUE KEY upstream_connections_legacy_uq(legacy_account_id),
  INDEX upstream_connections_channel_idx(channel_id, status, updated_at DESC)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE upstream_connection_credentials (
  id VARCHAR(128) PRIMARY KEY,
  connection_id VARCHAR(128) NOT NULL,
  credential_identity_hmac VARCHAR(64) NOT NULL,
  encrypted_payload LONGBLOB NOT NULL,
  payload_nonce VARBINARY(64) NOT NULL,
  wrapped_data_key VARBINARY(128) NOT NULL,
  wrap_nonce VARBINARY(64) NOT NULL,
  encryption_version INT UNSIGNED NOT NULL DEFAULT 1,
  credential_revision BIGINT UNSIGNED NOT NULL DEFAULT 0,
  status VARCHAR(16) NOT NULL DEFAULT 'active',
  created_at CHAR(24) NOT NULL,
  updated_at CHAR(24) NOT NULL,
  CONSTRAINT upstream_connection_credentials_version_chk CHECK (encryption_version = 1),
  CONSTRAINT upstream_connection_credentials_status_chk CHECK (status IN ('active', 'invalid', 'revoked')),
  CONSTRAINT upstream_connection_credentials_connection_fk FOREIGN KEY (connection_id) REFERENCES upstream_connections(id) ON DELETE CASCADE,
  UNIQUE KEY upstream_connection_credentials_connection_uq(connection_id),
  UNIQUE KEY upstream_connection_credentials_identity_uq(credential_identity_hmac)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE upstream_connection_models (
  connection_id VARCHAR(128) NOT NULL,
  model_id VARCHAR(128) NOT NULL,
  upstream_name VARCHAR(160) NOT NULL,
  enabled TINYINT UNSIGNED NOT NULL DEFAULT 1,
  capabilities_json LONGTEXT NOT NULL,
  discovered_at CHAR(24) NOT NULL,
  PRIMARY KEY (connection_id, model_id),
  CONSTRAINT upstream_connection_models_enabled_chk CHECK (enabled IN (0, 1)),
  CONSTRAINT upstream_connection_models_connection_fk FOREIGN KEY (connection_id) REFERENCES upstream_connections(id) ON DELETE CASCADE,
  CONSTRAINT upstream_connection_models_model_fk FOREIGN KEY (model_id) REFERENCES models(id) ON DELETE CASCADE,
  INDEX upstream_connection_models_model_idx(model_id, enabled, discovered_at DESC)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

INSERT INTO models(id,public_name,display_name,enabled,discovered_at,created_at)
VALUES
  ('model_builtin_gpt_image_2_5_flare','gpt-image-2.5-flare','GPT Image 2.5 Flare',1,NULL,'2026-09-17T00:00:00.000Z'),
  ('model_builtin_gpt_image_2_5_sunburst','gpt-image-2.5-sunburst','GPT Image 2.5 Sunburst',1,NULL,'2026-09-17T00:00:00.000Z'),
  ('model_builtin_gpt_image_2','gpt-image-2','GPT Image 2',1,NULL,'2026-09-17T00:00:00.000Z'),
  ('model_builtin_gpt_image_1','gpt-image-1','GPT Image 1',1,NULL,'2026-09-17T00:00:00.000Z')
ON DUPLICATE KEY UPDATE id=id;

CREATE TABLE identity_model_policies (
  identity_id VARCHAR(128) PRIMARY KEY,
  mode VARCHAR(16) NOT NULL,
  revision BIGINT UNSIGNED NOT NULL DEFAULT 0,
  grant_set_digest CHAR(64) NOT NULL,
  integrity_hmac CHAR(43) NOT NULL,
  updated_by VARCHAR(128) NOT NULL,
  updated_at CHAR(24) NOT NULL,
  CONSTRAINT identity_model_policies_mode_chk CHECK (mode IN ('selected', 'all_enabled')),
  CONSTRAINT identity_model_policies_identity_fk FOREIGN KEY (identity_id) REFERENCES identities(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE identity_model_grants (
  identity_id VARCHAR(128) NOT NULL,
  public_model_id VARCHAR(128) NOT NULL,
  PRIMARY KEY(identity_id,public_model_id),
  CONSTRAINT identity_model_grants_policy_fk FOREIGN KEY (identity_id) REFERENCES identity_model_policies(identity_id) ON DELETE CASCADE,
  CONSTRAINT identity_model_grants_model_fk FOREIGN KEY (public_model_id) REFERENCES models(id) ON DELETE CASCADE,
  INDEX identity_model_grants_model_idx(public_model_id,identity_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE model_attempt_admissions (
  attempt_id VARCHAR(128) PRIMARY KEY,
  identity_id VARCHAR(128) NOT NULL,
  public_model_id VARCHAR(128) NOT NULL,
  policy_revision BIGINT UNSIGNED NOT NULL,
  request_id VARCHAR(128) NOT NULL,
  admitted_at CHAR(24) NOT NULL,
  INDEX model_attempt_admissions_identity_idx(identity_id,admitted_at),
  CONSTRAINT model_attempt_admissions_identity_fk FOREIGN KEY (identity_id) REFERENCES identities(id),
  CONSTRAINT model_attempt_admissions_model_fk FOREIGN KEY (public_model_id) REFERENCES models(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

INSERT INTO identity_model_policies(identity_id,mode,revision,grant_set_digest,integrity_hmac,updated_by,updated_at)
SELECT id,'selected',0,'','','migration',updated_at FROM identities WHERE role='member' AND status<>'deleted';
INSERT INTO identity_model_grants(identity_id,public_model_id)
SELECT p.identity_id,m.id FROM identity_model_policies p CROSS JOIN models m WHERE m.enabled=1;
INSERT INTO transaction_gates(gate_key,revision) VALUES ('model_access_migration',0)
ON DUPLICATE KEY UPDATE gate_key='model_access_migration';

CREATE TABLE model_quota_units (
  model_id VARCHAR(128) PRIMARY KEY,
  quota_unit VARCHAR(8) NOT NULL,
  CONSTRAINT model_quota_units_unit_chk CHECK (quota_unit = 'image'),
  CONSTRAINT model_quota_units_model_fk FOREIGN KEY (model_id) REFERENCES models(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE member_image_balances (
  identity_id VARCHAR(128) NOT NULL,
  public_model_id VARCHAR(128) NOT NULL,
  available_images BIGINT NOT NULL DEFAULT 0,
  reserved_images BIGINT NOT NULL DEFAULT 0,
  consumed_images BIGINT NOT NULL DEFAULT 0,
  revision BIGINT UNSIGNED NOT NULL DEFAULT 0,
  last_ledger_hmac VARCHAR(128) NOT NULL DEFAULT '',
  integrity_hmac VARCHAR(128) NOT NULL,
  PRIMARY KEY (identity_id, public_model_id),
  CONSTRAINT member_image_balances_counts_chk CHECK (available_images >= 0 AND reserved_images >= 0 AND consumed_images >= 0),
  CONSTRAINT member_image_balances_identity_fk FOREIGN KEY (identity_id) REFERENCES identities(id) ON DELETE CASCADE,
  CONSTRAINT member_image_balances_model_fk FOREIGN KEY (public_model_id) REFERENCES models(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE image_quota_reservations (
  id VARCHAR(128) PRIMARY KEY,
  identity_id VARCHAR(128) NOT NULL,
  api_key_id VARCHAR(128) NOT NULL,
  public_model_id VARCHAR(128) NOT NULL,
  request_id VARCHAR(128) NOT NULL,
  reserved_images BIGINT NOT NULL,
  confirmed_images BIGINT NOT NULL DEFAULT 0,
  released_images BIGINT NOT NULL DEFAULT 0,
  status VARCHAR(24) NOT NULL,
  revision BIGINT UNSIGNED NOT NULL DEFAULT 0,
  integrity_hmac VARCHAR(128) NOT NULL,
  created_at CHAR(24) NOT NULL,
  expires_at CHAR(24) NOT NULL,
  settled_at CHAR(24),
  UNIQUE KEY image_quota_request_unique(identity_id, api_key_id, request_id, public_model_id),
  CONSTRAINT image_quota_reservation_counts_chk CHECK (reserved_images > 0 AND confirmed_images >= 0 AND released_images >= 0),
  CONSTRAINT image_quota_reservation_status_chk CHECK (status IN ('reserved','settled')),
  CONSTRAINT image_quota_reservation_balance_fk FOREIGN KEY (identity_id, public_model_id)
    REFERENCES member_image_balances(identity_id, public_model_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

CREATE TABLE image_quota_ledger (
  id VARCHAR(128) PRIMARY KEY,
  identity_id VARCHAR(128) NOT NULL,
  public_model_id VARCHAR(128) NOT NULL,
  reservation_id VARCHAR(128),
  child_id VARCHAR(128),
  kind VARCHAR(16) NOT NULL,
  amount_images BIGINT NOT NULL,
  produced_images BIGINT NOT NULL DEFAULT 0,
  delivery_state VARCHAR(24) NOT NULL DEFAULT 'none',
  actor VARCHAR(128) NOT NULL,
  reason VARCHAR(512) NOT NULL,
  previous_hmac VARCHAR(128) NOT NULL,
  integrity_hmac VARCHAR(128) NOT NULL,
  created_at CHAR(24) NOT NULL,
  UNIQUE KEY image_quota_ledger_child_unique(reservation_id, child_id, kind),
  INDEX image_quota_ledger_subject_idx(identity_id, public_model_id, created_at),
  CONSTRAINT image_quota_ledger_kind_chk CHECK (kind IN ('adjust','reserve','dispatch','confirm','release','delivery')),
  CONSTRAINT image_quota_ledger_count_chk CHECK (produced_images >= 0),
  CONSTRAINT image_quota_ledger_balance_fk FOREIGN KEY (identity_id, public_model_id)
    REFERENCES member_image_balances(identity_id, public_model_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;

INSERT INTO transaction_gates(gate_key,revision) VALUES ('image_quota_migration',0)
ON DUPLICATE KEY UPDATE gate_key='image_quota_migration';
