CREATE TABLE schema_migrations (
  version INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  checksum_sha256 TEXT NOT NULL CHECK (length(checksum_sha256) = 64),
  applied_at TEXT NOT NULL
) STRICT;

CREATE TABLE identities (
  id TEXT PRIMARY KEY,
  email TEXT NOT NULL,
  display_name TEXT NOT NULL,
  password_hash TEXT NOT NULL,
  role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled', 'deleted')),
  can_consume_model INTEGER NOT NULL DEFAULT 0 CHECK (can_consume_model IN (0, 1)),
  password_change_required INTEGER NOT NULL DEFAULT 1 CHECK (password_change_required IN (0, 1)),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
  integrity_hmac TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX identities_active_email_uq
  ON identities(email) WHERE status <> 'deleted';
CREATE INDEX identities_seat_idx ON identities(status, can_consume_model);

CREATE TABLE transaction_gates (
  gate_key TEXT PRIMARY KEY,
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0)
) STRICT;
INSERT INTO transaction_gates(gate_key, revision) VALUES ('member_seat', 0), ('audit_log', 0);

CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL,
  token_hash TEXT NOT NULL UNIQUE,
  expires_at TEXT NOT NULL,
  integrity_hmac TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY (identity_id) REFERENCES identities(id) ON DELETE CASCADE
) STRICT;
CREATE INDEX sessions_token_idx ON sessions(token_hash);

CREATE TABLE api_keys (
  id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL,
  name TEXT NOT NULL,
  key_hash TEXT NOT NULL UNIQUE,
  key_prefix TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'suspended', 'revoked')),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
  integrity_hmac TEXT NOT NULL,
  created_at TEXT NOT NULL,
  last_used_at TEXT,
  FOREIGN KEY (identity_id) REFERENCES identities(id) ON DELETE CASCADE
) STRICT;
CREATE INDEX api_keys_hash_idx ON api_keys(key_hash);

CREATE TABLE user_balances (
  identity_id TEXT PRIMARY KEY,
  balance_tokens INTEGER NOT NULL DEFAULT 0,
  reserved_tokens INTEGER NOT NULL DEFAULT 0 CHECK (reserved_tokens >= 0),
  granted_tokens INTEGER NOT NULL DEFAULT 0 CHECK (granted_tokens >= 0),
  consumed_tokens INTEGER NOT NULL DEFAULT 0 CHECK (consumed_tokens >= 0),
  request_count INTEGER NOT NULL DEFAULT 0 CHECK (request_count >= 0),
  raw_tokens INTEGER NOT NULL DEFAULT 0 CHECK (raw_tokens >= 0),
  billed_tokens INTEGER NOT NULL DEFAULT 0 CHECK (billed_tokens >= 0),
  uncached_input INTEGER NOT NULL DEFAULT 0 CHECK (uncached_input >= 0),
  cached_input INTEGER NOT NULL DEFAULT 0 CHECK (cached_input >= 0),
  cache_write INTEGER NOT NULL DEFAULT 0 CHECK (cache_write >= 0),
  output_tokens INTEGER NOT NULL DEFAULT 0 CHECK (output_tokens >= 0),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
  last_ledger_hmac TEXT NOT NULL DEFAULT '',
  integrity_hmac TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (identity_id) REFERENCES identities(id) ON DELETE CASCADE
) STRICT;

CREATE TABLE ledger_entries (
  id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  amount_tokens INTEGER NOT NULL,
  uncached_input INTEGER NOT NULL DEFAULT 0 CHECK (uncached_input >= 0),
  cached_input INTEGER NOT NULL DEFAULT 0 CHECK (cached_input >= 0),
  cache_write INTEGER NOT NULL DEFAULT 0 CHECK (cache_write >= 0),
  output_tokens INTEGER NOT NULL DEFAULT 0 CHECK (output_tokens >= 0),
  uncovered_tokens INTEGER NOT NULL DEFAULT 0 CHECK (uncovered_tokens >= 0),
  raw_tokens INTEGER NOT NULL DEFAULT 0 CHECK (raw_tokens >= 0),
  billed_tokens INTEGER NOT NULL DEFAULT 0 CHECK (billed_tokens >= 0),
  multiplier_micros INTEGER NOT NULL DEFAULT 1000000 CHECK (multiplier_micros > 0),
  reference_id TEXT,
  description TEXT NOT NULL,
  protocol TEXT NOT NULL DEFAULT '',
  model TEXT NOT NULL DEFAULT '',
  api_key_id TEXT,
  runner_id TEXT,
  previous_entry_hmac TEXT NOT NULL,
  integrity_hmac TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY (identity_id) REFERENCES identities(id),
  FOREIGN KEY (api_key_id) REFERENCES api_keys(id) ON DELETE SET NULL,
  UNIQUE (identity_id, kind, reference_id)
) STRICT;
CREATE INDEX ledger_identity_time_idx ON ledger_entries(identity_id, created_at DESC);

CREATE TABLE money_balances (
  identity_id TEXT PRIMARY KEY,
  currency TEXT NOT NULL CHECK (currency IN ('CNY', 'USD')),
  balance_nanos INTEGER NOT NULL,
  credited_nanos INTEGER NOT NULL CHECK (credited_nanos >= 0),
  debited_nanos INTEGER NOT NULL CHECK (debited_nanos >= 0),
  revision INTEGER NOT NULL CHECK (revision >= 0),
  last_ledger_hmac TEXT NOT NULL,
  integrity_hmac TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (identity_id) REFERENCES identities(id) ON DELETE CASCADE
) STRICT;

CREATE TABLE money_ledger_entries (
  id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL,
  currency TEXT NOT NULL CHECK (currency IN ('CNY', 'USD')),
  kind TEXT NOT NULL CHECK (kind IN ('grant', 'charge', 'anomaly', 'request_failed', 'adjustment', 'refund')),
  amount_nanos INTEGER NOT NULL,
  balance_revision INTEGER NOT NULL CHECK (balance_revision > 0),
  reference_id TEXT NOT NULL,
  billing_status TEXT NOT NULL CHECK (billing_status IN ('not_applicable', 'charged', 'billing_error')),
  details_json TEXT NOT NULL,
  previous_entry_hmac TEXT NOT NULL,
  integrity_hmac TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY (identity_id) REFERENCES identities(id),
  UNIQUE (identity_id, reference_id),
  UNIQUE (identity_id, balance_revision)
) STRICT;
CREATE INDEX money_ledger_identity_time_idx
  ON money_ledger_entries(identity_id, created_at DESC);

CREATE TABLE quota_reservations (
  id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL,
  api_key_id TEXT NOT NULL,
  request_id TEXT NOT NULL UNIQUE,
  reserved_tokens INTEGER NOT NULL CHECK (reserved_tokens > 0),
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'released', 'settled')),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
  integrity_hmac TEXT NOT NULL,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  settled_at TEXT,
  FOREIGN KEY (identity_id) REFERENCES identities(id),
  FOREIGN KEY (api_key_id) REFERENCES api_keys(id)
) STRICT;
CREATE INDEX quota_reservations_identity_status_idx
  ON quota_reservations(identity_id, status, expires_at);

CREATE TABLE vouchers (
  id TEXT PRIMARY KEY,
  code_hash TEXT NOT NULL UNIQUE,
  code_prefix TEXT NOT NULL,
  name TEXT NOT NULL,
  quota_tokens INTEGER NOT NULL CHECK (quota_tokens > 0),
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled', 'redeemed')),
  max_redemptions INTEGER NOT NULL DEFAULT 1 CHECK (max_redemptions > 0),
  redeemed_count INTEGER NOT NULL DEFAULT 0 CHECK (redeemed_count >= 0),
  expires_at TEXT,
  created_by TEXT NOT NULL,
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
  integrity_hmac TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY (created_by) REFERENCES identities(id)
) STRICT;

CREATE TABLE voucher_deliveries (
  id TEXT PRIMARY KEY,
  voucher_id TEXT NOT NULL,
  identity_id TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'redeemed')),
  delivered_at TEXT NOT NULL,
  redeemed_at TEXT,
  UNIQUE (voucher_id, identity_id),
  FOREIGN KEY (voucher_id) REFERENCES vouchers(id) ON DELETE CASCADE,
  FOREIGN KEY (identity_id) REFERENCES identities(id) ON DELETE CASCADE
) STRICT;
CREATE INDEX voucher_deliveries_identity_idx ON voucher_deliveries(identity_id, delivered_at DESC);

CREATE TABLE voucher_redemptions (
  id TEXT PRIMARY KEY,
  voucher_id TEXT NOT NULL,
  identity_id TEXT NOT NULL,
  amount_tokens INTEGER NOT NULL CHECK (amount_tokens > 0),
  ledger_entry_id TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL,
  UNIQUE (voucher_id, identity_id),
  FOREIGN KEY (voucher_id) REFERENCES vouchers(id),
  FOREIGN KEY (identity_id) REFERENCES identities(id),
  FOREIGN KEY (ledger_entry_id) REFERENCES ledger_entries(id)
) STRICT;

CREATE TABLE quota_requests (
  id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL,
  amount_nanos INTEGER NOT NULL CHECK (amount_nanos > 0),
  reason TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'rejected')),
  review_note TEXT NOT NULL DEFAULT '',
  reviewed_by TEXT,
  reviewed_at TEXT,
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
  integrity_hmac TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY (identity_id) REFERENCES identities(id),
  FOREIGN KEY (reviewed_by) REFERENCES identities(id)
) STRICT;
CREATE INDEX quota_requests_identity_idx ON quota_requests(identity_id, created_at DESC);
CREATE INDEX quota_requests_status_idx ON quota_requests(status, created_at DESC);
CREATE UNIQUE INDEX quota_requests_one_pending_idx ON quota_requests(identity_id) WHERE status = 'pending';

CREATE TABLE admin_quota_adjustments (
  id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL,
  admin_identity_id TEXT NOT NULL,
  amount_tokens INTEGER NOT NULL CHECK (amount_tokens > 0),
  reason TEXT NOT NULL,
  ledger_entry_id TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL,
  FOREIGN KEY (identity_id) REFERENCES identities(id),
  FOREIGN KEY (admin_identity_id) REFERENCES identities(id),
  FOREIGN KEY (ledger_entry_id) REFERENCES ledger_entries(id)
) STRICT;

CREATE TABLE runtime_settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
  integrity_hmac TEXT NOT NULL,
  updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE runner_enrollments (
  id TEXT PRIMARY KEY,
  token_hash TEXT NOT NULL UNIQUE,
  token_prefix TEXT NOT NULL,
  runner_name TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'used', 'expired', 'cancelled')),
  expires_at TEXT NOT NULL,
  used_at TEXT,
  runner_id TEXT,
  created_by TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY (created_by) REFERENCES identities(id)
) STRICT;
CREATE INDEX runner_enrollments_token_idx ON runner_enrollments(token_hash);

CREATE TABLE runners (
  id TEXT PRIMARY KEY,
  enrollment_id TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  credential_hash TEXT NOT NULL UNIQUE,
  enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  version TEXT NOT NULL,
  protocol_version INTEGER NOT NULL CHECK (protocol_version > 0),
  platform TEXT NOT NULL,
  architecture TEXT NOT NULL,
  max_inflight INTEGER NOT NULL DEFAULT 1 CHECK (max_inflight > 0),
  inflight INTEGER NOT NULL DEFAULT 0 CHECK (inflight >= 0 AND inflight <= max_inflight),
  recent_request_count INTEGER NOT NULL DEFAULT 0 CHECK (recent_request_count >= 0),
  recent_error_count INTEGER NOT NULL DEFAULT 0 CHECK (recent_error_count >= 0),
  latency_ms INTEGER NOT NULL DEFAULT 0 CHECK (latency_ms >= 0),
  last_seen_at TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (enrollment_id) REFERENCES runner_enrollments(id)
) STRICT;
CREATE INDEX runners_available_idx ON runners(enabled, protocol_version, last_seen_at DESC);

CREATE TABLE upstream_accounts (
  id TEXT PRIMARY KEY,
  provider TEXT NOT NULL,
  subject_id TEXT NOT NULL,
  email TEXT NOT NULL,
  plan TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled', 'invalid')),
  last_success_runner_id TEXT,
  last_verified_at TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE (provider, subject_id),
  FOREIGN KEY (last_success_runner_id) REFERENCES runners(id) ON DELETE SET NULL
) STRICT;
CREATE INDEX upstream_accounts_status_idx ON upstream_accounts(provider, status, updated_at DESC);

CREATE TABLE upstream_credential_instances (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL,
  credential_identity_hmac TEXT NOT NULL UNIQUE,
  encrypted_payload BLOB NOT NULL,
  payload_nonce BLOB NOT NULL,
  wrapped_data_key BLOB NOT NULL,
  wrap_nonce BLOB NOT NULL,
  encryption_version INTEGER NOT NULL DEFAULT 1 CHECK (encryption_version = 1),
  credential_revision INTEGER NOT NULL DEFAULT 0 CHECK (credential_revision >= 0),
  expires_at TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'refreshing', 'invalid', 'revoked')),
  last_refreshed_at TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (account_id) REFERENCES upstream_accounts(id) ON DELETE CASCADE
) STRICT;
CREATE INDEX credential_instances_account_idx ON upstream_credential_instances(account_id, status, expires_at);

CREATE TABLE credential_refresh_leases (
  credential_instance_id TEXT PRIMARY KEY,
  lease_token_hash TEXT NOT NULL UNIQUE,
  expected_revision INTEGER NOT NULL CHECK (expected_revision >= 0),
  expires_at TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY (credential_instance_id) REFERENCES upstream_credential_instances(id) ON DELETE CASCADE
) STRICT;
CREATE INDEX credential_refresh_leases_expiry_idx ON credential_refresh_leases(expires_at);

CREATE TABLE models (
  id TEXT PRIMARY KEY,
  public_name TEXT NOT NULL UNIQUE,
  display_name TEXT NOT NULL DEFAULT '',
  enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  discovered_at TEXT,
  created_at TEXT NOT NULL
) STRICT;

CREATE TABLE account_models (
  account_id TEXT NOT NULL,
  model_id TEXT NOT NULL,
  upstream_name TEXT NOT NULL,
  discovered_at TEXT NOT NULL,
  PRIMARY KEY (account_id, model_id),
  FOREIGN KEY (account_id) REFERENCES upstream_accounts(id) ON DELETE CASCADE,
  FOREIGN KEY (model_id) REFERENCES models(id) ON DELETE CASCADE
) STRICT;
CREATE INDEX account_models_model_idx ON account_models(model_id, discovered_at DESC);

CREATE TABLE audit_events (
  id TEXT PRIMARY KEY,
  event_sequence INTEGER NOT NULL UNIQUE CHECK (event_sequence > 0),
  actor_identity_id TEXT,
  actor_role TEXT NOT NULL CHECK (actor_role IN ('owner', 'admin', 'member', 'system', 'runner')),
  action TEXT NOT NULL,
  target_type TEXT NOT NULL,
  target_id TEXT,
  outcome TEXT NOT NULL CHECK (outcome IN ('succeeded', 'failed')),
  previous_event_hmac TEXT NOT NULL CHECK (previous_event_hmac = '' OR length(previous_event_hmac) = 43),
  integrity_hmac TEXT NOT NULL CHECK (length(integrity_hmac) = 43),
  created_at TEXT NOT NULL
) STRICT;
CREATE INDEX audit_events_time_idx ON audit_events(created_at DESC, event_sequence DESC);
CREATE INDEX audit_events_actor_idx ON audit_events(actor_identity_id, event_sequence DESC);
CREATE INDEX audit_events_action_idx ON audit_events(action, event_sequence DESC);

CREATE TABLE security_state (
  key TEXT PRIMARY KEY,
  value BLOB NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 0),
  mac BLOB NOT NULL,
  updated_at TEXT NOT NULL
) STRICT;

-- 2.2.0 fresh-install schema; previous releases are not upgrade inputs.

ALTER TABLE ledger_entries ADD COLUMN requested_model TEXT;
ALTER TABLE ledger_entries ADD COLUMN processing_tier TEXT;
ALTER TABLE ledger_entries ADD COLUMN reasoning_effort TEXT;

ALTER TABLE ledger_entries ADD COLUMN client_request_id TEXT;
CREATE INDEX ledger_identity_client_request_idx
  ON ledger_entries(identity_id, api_key_id, client_request_id);

INSERT INTO transaction_gates(gate_key, revision) VALUES ('entity_quota', 0);

CREATE TABLE upstream_connections (
  id TEXT PRIMARY KEY,
  provider TEXT NOT NULL,
  channel_id TEXT NOT NULL,
  endpoint_profile TEXT NOT NULL,
  auth_scheme TEXT NOT NULL CHECK (auth_scheme IN ('api_key', 'oauth')),
  billing_mode TEXT NOT NULL CHECK (billing_mode IN ('usage', 'coding_plan', 'subscription')),
  display_name TEXT NOT NULL,
  legacy_account_id TEXT UNIQUE,
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled', 'invalid')),
  revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (legacy_account_id) REFERENCES upstream_accounts(id) ON DELETE CASCADE
) STRICT;
CREATE INDEX upstream_connections_channel_idx
  ON upstream_connections(channel_id, status, updated_at DESC);

CREATE TABLE upstream_connection_credentials (
  id TEXT PRIMARY KEY,
  connection_id TEXT NOT NULL UNIQUE,
  credential_identity_hmac TEXT NOT NULL UNIQUE,
  encrypted_payload BLOB NOT NULL,
  payload_nonce BLOB NOT NULL,
  wrapped_data_key BLOB NOT NULL,
  wrap_nonce BLOB NOT NULL,
  encryption_version INTEGER NOT NULL DEFAULT 1 CHECK (encryption_version = 1),
  credential_revision INTEGER NOT NULL DEFAULT 0 CHECK (credential_revision >= 0),
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'invalid', 'revoked')),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (connection_id) REFERENCES upstream_connections(id) ON DELETE CASCADE
) STRICT;

CREATE TABLE upstream_connection_models (
  connection_id TEXT NOT NULL,
  model_id TEXT NOT NULL,
  upstream_name TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  capabilities_json TEXT NOT NULL DEFAULT '{}',
  discovered_at TEXT NOT NULL,
  PRIMARY KEY (connection_id, model_id),
  FOREIGN KEY (connection_id) REFERENCES upstream_connections(id) ON DELETE CASCADE,
  FOREIGN KEY (model_id) REFERENCES models(id) ON DELETE CASCADE
) STRICT;
CREATE INDEX upstream_connection_models_model_idx
  ON upstream_connection_models(model_id, enabled, discovered_at DESC);

INSERT INTO models(id,public_name,display_name,enabled,discovered_at,created_at)
VALUES
  ('model_builtin_gpt_image_2_5_flare','gpt-image-2.5-flare','GPT Image 2.5 Flare',1,NULL,'2026-09-17T00:00:00.000Z'),
  ('model_builtin_gpt_image_2_5_sunburst','gpt-image-2.5-sunburst','GPT Image 2.5 Sunburst',1,NULL,'2026-09-17T00:00:00.000Z'),
  ('model_builtin_gpt_image_2','gpt-image-2','GPT Image 2',1,NULL,'2026-09-17T00:00:00.000Z'),
  ('model_builtin_gpt_image_1','gpt-image-1','GPT Image 1',1,NULL,'2026-09-17T00:00:00.000Z')
ON CONFLICT(public_name) DO NOTHING;

CREATE TABLE identity_model_policies (
  identity_id TEXT PRIMARY KEY,
  mode TEXT NOT NULL CHECK (mode IN ('selected', 'all_enabled')),
  revision INTEGER NOT NULL CHECK (revision >= 0),
  grant_set_digest TEXT NOT NULL,
  integrity_hmac TEXT NOT NULL,
  updated_by TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (identity_id) REFERENCES identities(id) ON DELETE CASCADE
) STRICT;

CREATE TABLE identity_model_grants (
  identity_id TEXT NOT NULL,
  public_model_id TEXT NOT NULL,
  PRIMARY KEY (identity_id, public_model_id),
  FOREIGN KEY (identity_id) REFERENCES identity_model_policies(identity_id) ON DELETE CASCADE,
  FOREIGN KEY (public_model_id) REFERENCES models(id) ON DELETE CASCADE
) STRICT;
CREATE INDEX identity_model_grants_model_idx ON identity_model_grants(public_model_id,identity_id);

CREATE TABLE model_attempt_admissions (
  attempt_id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL,
  public_model_id TEXT NOT NULL,
  policy_revision INTEGER NOT NULL,
  request_id TEXT NOT NULL,
  admitted_at TEXT NOT NULL,
  FOREIGN KEY (identity_id) REFERENCES identities(id),
  FOREIGN KEY (public_model_id) REFERENCES models(id)
) STRICT;
CREATE INDEX model_attempt_admissions_identity_idx ON model_attempt_admissions(identity_id,admitted_at);

INSERT INTO identity_model_policies(identity_id,mode,revision,grant_set_digest,integrity_hmac,updated_by,updated_at)
SELECT id,'selected',0,'','', 'migration',updated_at FROM identities WHERE role='member' AND status<>'deleted';
INSERT INTO identity_model_grants(identity_id,public_model_id)
SELECT p.identity_id,m.id FROM identity_model_policies p CROSS JOIN models m WHERE m.enabled=1;

CREATE TABLE model_quota_units (
  model_id TEXT PRIMARY KEY REFERENCES models(id) ON DELETE CASCADE,
  quota_unit TEXT NOT NULL CHECK (quota_unit = 'image')
) STRICT;

CREATE TABLE member_image_balances (
  identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
  public_model_id TEXT NOT NULL REFERENCES models(id) ON DELETE CASCADE,
  available_images INTEGER NOT NULL DEFAULT 0 CHECK (available_images >= 0),
  reserved_images INTEGER NOT NULL DEFAULT 0 CHECK (reserved_images >= 0),
  consumed_images INTEGER NOT NULL DEFAULT 0 CHECK (consumed_images >= 0),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
  last_ledger_hmac TEXT NOT NULL DEFAULT '',
  integrity_hmac TEXT NOT NULL,
  PRIMARY KEY (identity_id, public_model_id)
) STRICT;

CREATE TABLE image_quota_reservations (
  id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL,
  api_key_id TEXT NOT NULL,
  public_model_id TEXT NOT NULL,
  request_id TEXT NOT NULL,
  reserved_images INTEGER NOT NULL CHECK (reserved_images > 0),
  confirmed_images INTEGER NOT NULL DEFAULT 0 CHECK (confirmed_images >= 0),
  released_images INTEGER NOT NULL DEFAULT 0 CHECK (released_images >= 0),
  status TEXT NOT NULL CHECK (status IN ('reserved','settled')),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
  integrity_hmac TEXT NOT NULL,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  settled_at TEXT,
  UNIQUE(identity_id, api_key_id, request_id, public_model_id),
  FOREIGN KEY(identity_id, public_model_id)
    REFERENCES member_image_balances(identity_id, public_model_id)
) STRICT;

CREATE TABLE image_quota_ledger (
  id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL,
  public_model_id TEXT NOT NULL,
  reservation_id TEXT,
  child_id TEXT,
  kind TEXT NOT NULL CHECK (kind IN ('adjust','reserve','dispatch','confirm','release','delivery')),
  amount_images INTEGER NOT NULL,
  produced_images INTEGER NOT NULL DEFAULT 0 CHECK (produced_images >= 0),
  delivery_state TEXT NOT NULL DEFAULT 'none',
  actor TEXT NOT NULL,
  reason TEXT NOT NULL,
  previous_hmac TEXT NOT NULL,
  integrity_hmac TEXT NOT NULL,
  created_at TEXT NOT NULL,
  UNIQUE(reservation_id, child_id, kind),
  FOREIGN KEY(identity_id, public_model_id)
    REFERENCES member_image_balances(identity_id, public_model_id)
) STRICT;
CREATE INDEX image_quota_ledger_subject_idx
  ON image_quota_ledger(identity_id, public_model_id, created_at);
