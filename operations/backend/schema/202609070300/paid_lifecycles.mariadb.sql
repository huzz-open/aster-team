CREATE TABLE IF NOT EXISTS commercial_paid_lifecycle_sources (
    fulfillment_id VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL PRIMARY KEY,
    source_namespace VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    kind VARCHAR(24) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    source_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    source_record_kind VARCHAR(24) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    source_record_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    source_record_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    document_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    snapshot_json JSON NOT NULL,
    content_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    created_at DATETIME(6) NOT NULL,
    UNIQUE KEY uq_paid_lifecycle_source (source_namespace, source_id),
    CONSTRAINT fk_paid_lifecycle_fulfillment FOREIGN KEY (fulfillment_id) REFERENCES commercial_paid_fulfillments(id),
    CONSTRAINT chk_paid_lifecycle_namespace CHECK (source_namespace IN ('paid','trial')),
    CONSTRAINT chk_paid_lifecycle_kind CHECK (kind IN ('renewal','upgrade','trial_conversion')),
    CONSTRAINT chk_paid_lifecycle_record_kind CHECK (source_record_kind IN ('paid_fulfillment','paid_transfer','trial_issuance'))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

ALTER TABLE trials
    ADD COLUMN IF NOT EXISTS converted_commercial_order_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NULL,
    ADD CONSTRAINT fk_trials_converted_commercial_order FOREIGN KEY (converted_commercial_order_id) REFERENCES commercial_orders(id);

ALTER TABLE commercial_installation_requests
    DROP CONSTRAINT IF EXISTS chk_commercial_installation_request_usage,
    ADD CONSTRAINT chk_commercial_installation_request_usage CHECK (usage_kind IN ('initial','transfer','successor'));
