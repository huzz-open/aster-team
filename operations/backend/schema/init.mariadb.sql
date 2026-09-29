-- Aster Team Operations MariaDB initialization baseline.
CREATE TABLE operators (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    email VARCHAR(320) NOT NULL,
    normalized_email VARCHAR(320) NOT NULL,
    display_name VARCHAR(120) NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    status VARCHAR(32) NOT NULL,
    password_change_required BOOLEAN NOT NULL DEFAULT TRUE,
    created_at DATETIME(6) NOT NULL,
    updated_at DATETIME(6) NOT NULL,
    UNIQUE KEY uq_operators_normalized_email (normalized_email),
    CONSTRAINT chk_operators_status CHECK (status IN ('active', 'disabled'))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE operator_sessions (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    operator_id VARCHAR(64) NOT NULL,
    token_hash BINARY(32) NOT NULL,
    csrf_hash BINARY(32) NOT NULL,
    expires_at DATETIME(6) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    last_seen_at DATETIME(6) NOT NULL,
    UNIQUE KEY uq_operator_sessions_token_hash (token_hash),
    KEY idx_operator_sessions_operator (operator_id),
    KEY idx_operator_sessions_expires (expires_at),
    CONSTRAINT fk_operator_sessions_operator FOREIGN KEY (operator_id) REFERENCES operators(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE customers (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    name VARCHAR(160) NOT NULL,
    legal_name VARCHAR(200) NOT NULL DEFAULT '',
    status VARCHAR(32) NOT NULL,
    contact_name VARCHAR(120) NOT NULL DEFAULT '',
    contact_email VARCHAR(320) NOT NULL DEFAULT '',
    contact_phone VARCHAR(64) NOT NULL DEFAULT '',
    contact_wechat VARCHAR(120) NOT NULL DEFAULT '',
    notes TEXT NOT NULL,
    created_at DATETIME(6) NOT NULL,
    updated_at DATETIME(6) NOT NULL,
    KEY idx_customers_status_id (status, id),
    CONSTRAINT chk_customers_status CHECK (status IN ('lead', 'active', 'inactive'))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE audit_events (
    id VARCHAR(96) NOT NULL PRIMARY KEY,
    operator_id VARCHAR(64) NOT NULL,
    action VARCHAR(120) NOT NULL,
    resource_type VARCHAR(80) NOT NULL,
    resource_id VARCHAR(96) NOT NULL,
    payload_json JSON NOT NULL,
    created_at DATETIME(6) NOT NULL,
    KEY idx_audit_events_created (created_at),
    KEY idx_audit_events_resource (resource_type, resource_id, created_at),
    CONSTRAINT fk_audit_events_operator FOREIGN KEY (operator_id) REFERENCES operators(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE operator_permissions (
    operator_id VARCHAR(64) NOT NULL,
    permission_code VARCHAR(80) NOT NULL,
    granted_at DATETIME(6) NOT NULL,
    PRIMARY KEY (operator_id, permission_code),
    KEY idx_operator_permissions_code (permission_code, operator_id),
    CONSTRAINT fk_operator_permissions_operator FOREIGN KEY (operator_id) REFERENCES operators(id) ON DELETE CASCADE,
    CONSTRAINT chk_operator_permissions_release CHECK (permission_code IN (
        'release.read',
        'release.build',
        'release.download',
        'release.publish.request',
        'release.publish.approve',
        'release.publish.execute'
    ))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE contacts (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    customer_id VARCHAR(64) NOT NULL,
    name VARCHAR(120) NOT NULL,
    email VARCHAR(320) NOT NULL DEFAULT '',
    phone VARCHAR(64) NOT NULL DEFAULT '',
    wechat VARCHAR(120) NOT NULL DEFAULT '',
    role_title VARCHAR(120) NOT NULL DEFAULT '',
    is_primary BOOLEAN NOT NULL DEFAULT FALSE,
    created_at DATETIME(6) NOT NULL,
    updated_at DATETIME(6) NOT NULL,
    KEY idx_contacts_customer_email (customer_id, email),
    CONSTRAINT fk_contacts_customer FOREIGN KEY (customer_id) REFERENCES customers(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE billing_profiles (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    customer_id VARCHAR(64) NOT NULL,
    invoice_title VARCHAR(200) NOT NULL DEFAULT '',
    tax_identifier VARCHAR(80) NOT NULL DEFAULT '',
    billing_email VARCHAR(320) NOT NULL DEFAULT '',
    address TEXT NOT NULL,
    created_at DATETIME(6) NOT NULL,
    updated_at DATETIME(6) NOT NULL,
    UNIQUE KEY uq_billing_profiles_customer (customer_id),
    CONSTRAINT fk_billing_profiles_customer FOREIGN KEY (customer_id) REFERENCES customers(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE products (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    code VARCHAR(80) NOT NULL,
    name VARCHAR(160) NOT NULL,
    status VARCHAR(32) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    updated_at DATETIME(6) NOT NULL,
    UNIQUE KEY uq_products_code (code),
    CONSTRAINT chk_products_status CHECK (status IN ('active', 'archived'))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE plans (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    product_id VARCHAR(64) NOT NULL,
    code VARCHAR(80) NOT NULL,
    name VARCHAR(160) NOT NULL,
    edition VARCHAR(80) NOT NULL,
    status VARCHAR(32) NOT NULL,
    features_json JSON NOT NULL,
    transfer_limit INT NOT NULL,
    member_seats_limit INT NOT NULL,
    seat_over_limit_grace_days INT NOT NULL,
    minimum_version VARCHAR(64) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    updated_at DATETIME(6) NOT NULL,
    UNIQUE KEY uq_plans_product_code (product_id, code),
    CONSTRAINT fk_plans_product FOREIGN KEY (product_id) REFERENCES products(id),
    CONSTRAINT chk_plans_status CHECK (status IN ('draft', 'active', 'archived')),
    CONSTRAINT chk_plans_limits CHECK (transfer_limit >= 0 AND member_seats_limit >= 0)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE price_versions (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    plan_id VARCHAR(64) NOT NULL,
    version_no INT NOT NULL,
    currency CHAR(3) NOT NULL,
    billing_cycle VARCHAR(32) NOT NULL,
    base_amount_minor BIGINT NOT NULL,
    included_member_seats INT NOT NULL,
    additional_member_seat_minor BIGINT NOT NULL,
    tax_mode VARCHAR(32) NOT NULL,
    published_at DATETIME(6) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    UNIQUE KEY uq_price_versions_plan_version (plan_id, version_no),
    CONSTRAINT fk_price_versions_plan FOREIGN KEY (plan_id) REFERENCES plans(id),
    CONSTRAINT chk_price_versions_values CHECK (base_amount_minor >= 0 AND included_member_seats >= 0 AND additional_member_seat_minor >= 0),
    CONSTRAINT chk_price_versions_cycle CHECK (billing_cycle IN ('month', 'year', 'one_time')),
    CONSTRAINT chk_price_versions_tax CHECK (tax_mode IN ('inclusive', 'exclusive', 'none'))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE orders (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    customer_id VARCHAR(64) NOT NULL,
    plan_id VARCHAR(64) NOT NULL,
    price_version_id VARCHAR(64) NOT NULL,
    contract_ref VARCHAR(128) NOT NULL DEFAULT '',
    status VARCHAR(40) NOT NULL,
    amount_minor BIGINT NOT NULL,
    currency CHAR(3) NOT NULL,
    starts_at DATETIME(6) NOT NULL,
    ends_at DATETIME(6) NOT NULL,
    notes TEXT NOT NULL,
    created_by VARCHAR(64) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    updated_at DATETIME(6) NOT NULL,
    KEY idx_orders_customer_created (customer_id, created_at),
    KEY idx_orders_status_created (status, created_at),
    KEY idx_orders_contract_ref (contract_ref),
    CONSTRAINT fk_orders_customer FOREIGN KEY (customer_id) REFERENCES customers(id),
    CONSTRAINT fk_orders_plan FOREIGN KEY (plan_id) REFERENCES plans(id),
    CONSTRAINT fk_orders_price_version FOREIGN KEY (price_version_id) REFERENCES price_versions(id),
    CONSTRAINT fk_orders_creator FOREIGN KEY (created_by) REFERENCES operators(id),
    CONSTRAINT chk_orders_status CHECK (status IN ('draft', 'pending_payment', 'fulfillment_pending', 'fulfilled', 'cancelled', 'refunded')),
    CONSTRAINT chk_orders_amount CHECK (amount_minor >= 0 AND ends_at > starts_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE order_items (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    order_id VARCHAR(64) NOT NULL,
    description VARCHAR(240) NOT NULL,
    quantity INT NOT NULL,
    unit_amount_minor BIGINT NOT NULL,
    price_snapshot_json JSON NOT NULL,
    created_at DATETIME(6) NOT NULL,
    KEY idx_order_items_order (order_id),
    CONSTRAINT fk_order_items_order FOREIGN KEY (order_id) REFERENCES orders(id),
    CONSTRAINT chk_order_items_values CHECK (quantity >= 1 AND unit_amount_minor >= 0)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE offline_payment_confirmations (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    operation_id VARCHAR(128) NOT NULL,
    order_id VARCHAR(64) NOT NULL,
    payment_reference VARCHAR(160) NOT NULL,
    amount_minor BIGINT NOT NULL,
    currency CHAR(3) NOT NULL,
    confirmed_by VARCHAR(64) NOT NULL,
    reviewed_by VARCHAR(64) NOT NULL,
    confirmed_at DATETIME(6) NOT NULL,
    notes TEXT NOT NULL,
    UNIQUE KEY uq_offline_payment_operation (operation_id),
    UNIQUE KEY uq_offline_payment_order (order_id),
    CONSTRAINT fk_offline_payment_order FOREIGN KEY (order_id) REFERENCES orders(id),
    CONSTRAINT fk_offline_payment_confirmer FOREIGN KEY (confirmed_by) REFERENCES operators(id),
    CONSTRAINT fk_offline_payment_reviewer FOREIGN KEY (reviewed_by) REFERENCES operators(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE refund_notes (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    order_id VARCHAR(64) NOT NULL,
    amount_minor BIGINT NOT NULL,
    reason TEXT NOT NULL,
    operator_id VARCHAR(64) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    KEY idx_refund_notes_order (order_id, created_at),
    CONSTRAINT fk_refund_notes_order FOREIGN KEY (order_id) REFERENCES orders(id),
    CONSTRAINT fk_refund_notes_operator FOREIGN KEY (operator_id) REFERENCES operators(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE trials (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    customer_id VARCHAR(64) NOT NULL,
    plan_id VARCHAR(64) NOT NULL,
    status VARCHAR(32) NOT NULL,
    starts_at DATETIME(6) NOT NULL,
    ends_at DATETIME(6) NOT NULL,
    member_seats INT NOT NULL,
    transfer_limit INT NOT NULL,
    approval_reason TEXT NOT NULL,
    approved_by VARCHAR(64) NOT NULL,
    converted_order_id VARCHAR(64),
    created_at DATETIME(6) NOT NULL,
    updated_at DATETIME(6) NOT NULL,
    KEY idx_trials_customer_status (customer_id, status),
    CONSTRAINT fk_trials_customer FOREIGN KEY (customer_id) REFERENCES customers(id),
    CONSTRAINT fk_trials_plan FOREIGN KEY (plan_id) REFERENCES plans(id),
    CONSTRAINT fk_trials_approver FOREIGN KEY (approved_by) REFERENCES operators(id),
    CONSTRAINT fk_trials_converted_order FOREIGN KEY (converted_order_id) REFERENCES orders(id),
    CONSTRAINT chk_trials_status CHECK (status IN ('approved', 'active', 'expired', 'converted', 'rejected')),
    CONSTRAINT chk_trials_limits CHECK (member_seats >= 0 AND transfer_limit >= 0 AND ends_at > starts_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE risk_notes (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    trial_id VARCHAR(64) NOT NULL,
    risk_level VARCHAR(32) NOT NULL,
    note TEXT NOT NULL,
    operator_id VARCHAR(64) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    KEY idx_risk_notes_trial (trial_id, created_at),
    CONSTRAINT fk_risk_notes_trial FOREIGN KEY (trial_id) REFERENCES trials(id),
    CONSTRAINT fk_risk_notes_operator FOREIGN KEY (operator_id) REFERENCES operators(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE license_records (
    license_id VARCHAR(128) NOT NULL PRIMARY KEY,
    operation_id VARCHAR(128) NOT NULL,
    customer_id VARCHAR(64) NOT NULL,
    source_type VARCHAR(32) NOT NULL,
    source_id VARCHAR(64) NOT NULL,
    customer_ref VARCHAR(128) NOT NULL,
    policy_json JSON NOT NULL,
    status VARCHAR(32) NOT NULL,
    transfer_limit INT NOT NULL,
    transfer_count INT NOT NULL DEFAULT 0,
    created_at DATETIME(6) NOT NULL,
    updated_at DATETIME(6) NOT NULL,
    UNIQUE KEY uq_license_records_source (source_type, source_id),
    UNIQUE KEY uq_license_records_operation (operation_id),
    KEY idx_license_records_customer (customer_id, created_at),
    CONSTRAINT fk_license_records_customer FOREIGN KEY (customer_id) REFERENCES customers(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE license_issuances (
    id VARCHAR(128) NOT NULL PRIMARY KEY,
    license_id VARCHAR(128) NOT NULL,
    request_id VARCHAR(128) NOT NULL,
    installation_id VARCHAR(128) NOT NULL,
    machine_fingerprint_sha256 CHAR(43) NOT NULL,
    transfer_sequence INT NOT NULL,
    document_json JSON NOT NULL,
    sha256 CHAR(64) NOT NULL,
    issued_by VARCHAR(64) NOT NULL,
    issued_at DATETIME(6) NOT NULL,
    expires_at DATETIME(6) NOT NULL,
    UNIQUE KEY uq_license_issuances_request (request_id),
    KEY idx_license_issuances_license (license_id, issued_at),
    KEY idx_license_issuances_sequence (license_id, transfer_sequence),
    CONSTRAINT fk_license_issuances_license FOREIGN KEY (license_id) REFERENCES license_records(license_id),
    CONSTRAINT fk_license_issuances_operator FOREIGN KEY (issued_by) REFERENCES operators(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE release_artifacts (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    version VARCHAR(64) NOT NULL,
    platform VARCHAR(40) NOT NULL,
    architecture VARCHAR(40) NOT NULL,
    object_key VARCHAR(512) NOT NULL,
    sha256 CHAR(64) NOT NULL,
    release_manifest_sha256 CHAR(64) NOT NULL,
    size_bytes BIGINT NOT NULL,
    signature_ref VARCHAR(512) NOT NULL DEFAULT '',
    source_commit_sha VARCHAR(64),
    github_run_id BIGINT,
    runtime_linkage VARCHAR(32) NOT NULL DEFAULT '',
    imported_by VARCHAR(64) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    UNIQUE KEY uq_release_artifact_target_hash (version, platform, architecture, sha256),
    UNIQUE KEY uq_release_artifact_object_key (object_key),
    KEY idx_release_artifacts_target (version, platform, architecture, created_at),
    CONSTRAINT fk_release_artifact_importer FOREIGN KEY (imported_by) REFERENCES operators(id),
    CONSTRAINT chk_release_artifact_hash CHECK (
        sha256 REGEXP '^[0-9a-f]{64}$' AND
        release_manifest_sha256 REGEXP '^[0-9a-f]{64}$' AND
        platform = 'linux' AND architecture = 'amd64' AND size_bytes > 0 AND
        runtime_linkage IN ('musl-static', 'unverified')
    )
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE release_tasks (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    version VARCHAR(64) NOT NULL,
    platform VARCHAR(40) NOT NULL,
    architecture VARCHAR(40) NOT NULL,
    mode VARCHAR(32) NOT NULL,
    github_repository VARCHAR(255) NOT NULL,
    workflow_file VARCHAR(255) NOT NULL,
    source_ref VARCHAR(255) NOT NULL,
    source_commit_sha VARCHAR(64) NOT NULL,
    free_distribution_id VARCHAR(64),
    free_license_sha256 CHAR(64),
    phase VARCHAR(64) NOT NULL,
    status VARCHAR(32) NOT NULL,
    error_code VARCHAR(80),
    active_dedup_key VARCHAR(192),
    retry_of_task_id VARCHAR(64),
    created_by VARCHAR(64) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    updated_at DATETIME(6) NOT NULL,
    started_at DATETIME(6),
    completed_at DATETIME(6),
    KEY idx_release_tasks_created (created_at, id),
    KEY idx_release_tasks_status (status, updated_at),
    KEY idx_release_tasks_source (source_commit_sha, version),
    UNIQUE KEY uq_release_tasks_active_dedup (active_dedup_key),
    CONSTRAINT fk_release_tasks_creator FOREIGN KEY (created_by) REFERENCES operators(id),
    CONSTRAINT fk_release_tasks_retry FOREIGN KEY (retry_of_task_id) REFERENCES release_tasks(id),
    CONSTRAINT chk_release_tasks_target CHECK (platform = 'linux' AND architecture = 'amd64'),
    CONSTRAINT chk_release_tasks_mode CHECK (mode IN ('verification', 'release')),
    CONSTRAINT chk_release_tasks_status CHECK (status IN ('pending_approval', 'queued', 'dispatching', 'in_progress', 'verifying', 'completed', 'failed', 'cancelled')),
    CONSTRAINT chk_release_tasks_commit CHECK (source_commit_sha REGEXP '^[0-9a-f]{40,64}$')
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE release_runs (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    release_task_id VARCHAR(64) NOT NULL,
    attempt INT NOT NULL,
    github_run_id BIGINT,
    github_run_number BIGINT,
    workflow_name VARCHAR(255) NOT NULL,
    head_branch VARCHAR(255) NOT NULL,
    head_sha VARCHAR(64) NOT NULL,
    status VARCHAR(32) NOT NULL,
    conclusion VARCHAR(32),
    html_url VARCHAR(512) NOT NULL,
    started_at DATETIME(6),
    completed_at DATETIME(6),
    synced_at DATETIME(6),
    created_at DATETIME(6) NOT NULL,
    UNIQUE KEY uq_release_runs_task_attempt (release_task_id, attempt),
    UNIQUE KEY uq_release_runs_github (github_run_id),
    KEY idx_release_runs_status (status, synced_at),
    CONSTRAINT fk_release_runs_task FOREIGN KEY (release_task_id) REFERENCES release_tasks(id) ON DELETE CASCADE,
    CONSTRAINT chk_release_runs_attempt CHECK (attempt > 0),
    CONSTRAINT chk_release_runs_status CHECK (status IN ('requested', 'waiting', 'pending', 'queued', 'in_progress', 'completed')),
    CONSTRAINT chk_release_runs_conclusion CHECK (conclusion IS NULL OR conclusion IN ('success', 'failure', 'cancelled', 'skipped', 'timed_out', 'action_required', 'neutral', 'stale', 'startup_failure'))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE release_run_jobs (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    release_run_id VARCHAR(64) NOT NULL,
    github_job_id BIGINT NOT NULL,
    sequence_no INT NOT NULL,
    name VARCHAR(255) NOT NULL,
    runner_name VARCHAR(255) NOT NULL DEFAULT '',
    status VARCHAR(32) NOT NULL,
    conclusion VARCHAR(32),
    html_url VARCHAR(512) NOT NULL,
    started_at DATETIME(6),
    completed_at DATETIME(6),
    UNIQUE KEY uq_release_run_jobs_github (github_job_id),
    UNIQUE KEY uq_release_run_jobs_sequence (release_run_id, sequence_no),
    KEY idx_release_run_jobs_run (release_run_id, sequence_no),
    CONSTRAINT fk_release_run_jobs_run FOREIGN KEY (release_run_id) REFERENCES release_runs(id) ON DELETE CASCADE,
    CONSTRAINT chk_release_run_jobs_sequence CHECK (sequence_no >= 0),
    CONSTRAINT chk_release_run_jobs_status CHECK (status IN ('requested', 'waiting', 'pending', 'queued', 'in_progress', 'completed'))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE release_run_steps (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    release_run_job_id VARCHAR(64) NOT NULL,
    github_step_number INT NOT NULL,
    sequence_no INT NOT NULL,
    name VARCHAR(255) NOT NULL,
    status VARCHAR(32) NOT NULL,
    conclusion VARCHAR(32),
    failure_summary TEXT NOT NULL,
    log_ref VARCHAR(512) NOT NULL DEFAULT '',
    started_at DATETIME(6),
    completed_at DATETIME(6),
    UNIQUE KEY uq_release_run_steps_number (release_run_job_id, github_step_number),
    KEY idx_release_run_steps_job (release_run_job_id, sequence_no),
    CONSTRAINT fk_release_run_steps_job FOREIGN KEY (release_run_job_id) REFERENCES release_run_jobs(id) ON DELETE CASCADE,
    CONSTRAINT chk_release_run_steps_sequence CHECK (sequence_no >= 0),
    CONSTRAINT chk_release_run_steps_status CHECK (status IN ('queued', 'in_progress', 'completed', 'pending'))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE release_task_artifacts (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    release_task_id VARCHAR(64) NOT NULL,
    release_run_id VARCHAR(64) NOT NULL,
    github_artifact_id BIGINT NOT NULL,
    name VARCHAR(255) NOT NULL,
    file_name VARCHAR(255) NOT NULL,
    size_bytes BIGINT NOT NULL,
    expires_at DATETIME(6),
    download_ref VARCHAR(512) NOT NULL DEFAULT '',
    github_digest_sha256 CHAR(64),
    sha256 CHAR(64),
    release_manifest_sha256 CHAR(64),
    signature_key_id VARCHAR(128) NOT NULL DEFAULT '',
    runtime_linkage VARCHAR(32) NOT NULL DEFAULT '',
    verification_status VARCHAR(32) NOT NULL,
    verification_error_code VARCHAR(80),
    release_artifact_id VARCHAR(64),
    verified_at DATETIME(6),
    created_at DATETIME(6) NOT NULL,
    UNIQUE KEY uq_release_task_artifacts_github (github_artifact_id),
    KEY idx_release_task_artifacts_task (release_task_id, created_at),
    CONSTRAINT fk_release_task_artifacts_task FOREIGN KEY (release_task_id) REFERENCES release_tasks(id) ON DELETE CASCADE,
    CONSTRAINT fk_release_task_artifacts_run FOREIGN KEY (release_run_id) REFERENCES release_runs(id) ON DELETE CASCADE,
    CONSTRAINT fk_release_task_artifacts_import FOREIGN KEY (release_artifact_id) REFERENCES release_artifacts(id),
    CONSTRAINT chk_release_task_artifacts_size CHECK (size_bytes >= 0),
    CONSTRAINT chk_release_task_artifacts_hash CHECK (
        (github_digest_sha256 IS NULL OR github_digest_sha256 REGEXP '^[0-9a-f]{64}$') AND
        (sha256 IS NULL OR sha256 REGEXP '^[0-9a-f]{64}$') AND
        (release_manifest_sha256 IS NULL OR release_manifest_sha256 REGEXP '^[0-9a-f]{64}$')
    ),
    CONSTRAINT chk_release_task_artifacts_verification CHECK (
        verification_status IN ('pending', 'verified', 'failed', 'unavailable') AND
        runtime_linkage IN ('', 'musl-static') AND
        (verification_status <> 'verified' OR runtime_linkage = 'musl-static')
    )
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE release_publish_requests (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    release_artifact_id VARCHAR(64) NOT NULL,
    version VARCHAR(64) NOT NULL,
    source_commit_sha VARCHAR(64) NOT NULL,
    tag_name VARCHAR(96) NOT NULL,
    status VARCHAR(32) NOT NULL,
    error_code VARCHAR(80),
    active_dedup_key VARCHAR(192),
    requested_by VARCHAR(64) NOT NULL,
    approved_by VARCHAR(64),
    executed_by VARCHAR(64),
    approval_comment VARCHAR(1000) NOT NULL DEFAULT '',
    github_release_id BIGINT,
    html_url VARCHAR(512) NOT NULL DEFAULT '',
    created_at DATETIME(6) NOT NULL,
    updated_at DATETIME(6) NOT NULL,
    approved_at DATETIME(6),
    published_at DATETIME(6),
    UNIQUE KEY uq_release_publish_active (active_dedup_key),
    UNIQUE KEY uq_release_publish_github (github_release_id),
    KEY idx_release_publish_status (status, updated_at),
    KEY idx_release_publish_artifact (release_artifact_id, created_at),
    CONSTRAINT fk_release_publish_artifact FOREIGN KEY (release_artifact_id) REFERENCES release_artifacts(id),
    CONSTRAINT fk_release_publish_requester FOREIGN KEY (requested_by) REFERENCES operators(id),
    CONSTRAINT fk_release_publish_approver FOREIGN KEY (approved_by) REFERENCES operators(id),
    CONSTRAINT fk_release_publish_executor FOREIGN KEY (executed_by) REFERENCES operators(id),
    CONSTRAINT chk_release_publish_commit CHECK (source_commit_sha REGEXP '^[0-9a-f]{40}$'),
    CONSTRAINT chk_release_publish_status CHECK (status IN ('requested', 'approved', 'rejected', 'publishing', 'published', 'failed'))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE release_publish_approvals (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    publish_request_id VARCHAR(64) NOT NULL,
    decision VARCHAR(32) NOT NULL,
    comment VARCHAR(1000) NOT NULL DEFAULT '',
    decided_by VARCHAR(64) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    UNIQUE KEY uq_release_publish_approval (publish_request_id),
    CONSTRAINT fk_release_publish_approval_request FOREIGN KEY (publish_request_id) REFERENCES release_publish_requests(id),
    CONSTRAINT fk_release_publish_approval_operator FOREIGN KEY (decided_by) REFERENCES operators(id),
    CONSTRAINT chk_release_publish_decision CHECK (decision IN ('approved', 'rejected'))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE delivery_records (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    customer_id VARCHAR(64) NOT NULL,
    order_id VARCHAR(64),
    trial_id VARCHAR(64),
    license_id VARCHAR(128) NOT NULL,
    release_artifact_id VARCHAR(64) NOT NULL,
    channel VARCHAR(80) NOT NULL,
    recipient VARCHAR(320) NOT NULL,
    receipt_object_key VARCHAR(512) NOT NULL DEFAULT '',
    receipt_sha256 CHAR(64) NOT NULL,
    delivered_at DATETIME(6),
    created_by VARCHAR(64) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    KEY idx_delivery_records_customer (customer_id, created_at),
    CONSTRAINT fk_delivery_customer FOREIGN KEY (customer_id) REFERENCES customers(id),
    CONSTRAINT fk_delivery_order FOREIGN KEY (order_id) REFERENCES orders(id),
    CONSTRAINT fk_delivery_trial FOREIGN KEY (trial_id) REFERENCES trials(id),
    CONSTRAINT fk_delivery_license FOREIGN KEY (license_id) REFERENCES license_records(license_id),
    CONSTRAINT fk_delivery_artifact FOREIGN KEY (release_artifact_id) REFERENCES release_artifacts(id),
    CONSTRAINT fk_delivery_creator FOREIGN KEY (created_by) REFERENCES operators(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE backup_history (
    id VARCHAR(64) NOT NULL PRIMARY KEY,
    kind VARCHAR(40) NOT NULL,
    object_ref VARCHAR(512) NOT NULL,
    sha256 CHAR(64) NOT NULL,
    size_bytes BIGINT NOT NULL,
    status VARCHAR(32) NOT NULL,
    operator_id VARCHAR(64) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    verified_at DATETIME(6),
    CONSTRAINT fk_backup_history_operator FOREIGN KEY (operator_id) REFERENCES operators(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
