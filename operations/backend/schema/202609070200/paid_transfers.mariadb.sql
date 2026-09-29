CREATE TABLE IF NOT EXISTS commercial_installation_requests (
    request_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL PRIMARY KEY,
    request_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    customer_id VARCHAR(64) NOT NULL,
    usage_kind VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    created_at DATETIME(6) NOT NULL,
    CONSTRAINT fk_commercial_installation_request_customer FOREIGN KEY (customer_id) REFERENCES customers(id),
    CONSTRAINT chk_commercial_installation_request_usage CHECK (usage_kind IN ('initial','transfer'))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

INSERT INTO commercial_installation_requests(request_id,request_sha256,customer_id,usage_kind,created_at)
SELECT request_id,request_sha256,customer_id,'initial',approved_at FROM commercial_paid_fulfillments
ON DUPLICATE KEY UPDATE
    request_sha256=IF(commercial_installation_requests.request_sha256=VALUES(request_sha256) AND BINARY commercial_installation_requests.customer_id=BINARY VALUES(customer_id) AND commercial_installation_requests.usage_kind=VALUES(usage_kind),commercial_installation_requests.request_sha256,NULL);

CREATE TABLE IF NOT EXISTS commercial_paid_transfers (
    id VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL PRIMARY KEY,
    operation_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    fulfillment_id VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    fulfillment_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    customer_id VARCHAR(64) NOT NULL,
    request_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    request_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    previous_document_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    transfer_sequence INT UNSIGNED NOT NULL,
    environment VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    snapshot_json JSON NOT NULL,
    content_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    approved_by VARCHAR(64) NOT NULL,
    approved_at DATETIME(6) NOT NULL,
    status VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    claims_json JSON NULL,
    document_json JSON NULL,
    document_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NULL,
    UNIQUE KEY uq_paid_transfer_operation (operation_id),
    UNIQUE KEY uq_paid_transfer_sequence (fulfillment_id, transfer_sequence),
    UNIQUE KEY uq_paid_transfer_request (request_id),
    CONSTRAINT fk_paid_transfer_fulfillment FOREIGN KEY (fulfillment_id) REFERENCES commercial_paid_fulfillments(id),
    CONSTRAINT fk_paid_transfer_customer FOREIGN KEY (customer_id) REFERENCES customers(id),
    CONSTRAINT fk_paid_transfer_actor FOREIGN KEY (approved_by) REFERENCES operators(id),
    CONSTRAINT chk_paid_transfer_environment CHECK (environment IN ('local', 'production')),
    CONSTRAINT chk_paid_transfer_sequence CHECK (transfer_sequence > 0 AND transfer_sequence <= 10000),
    CONSTRAINT chk_paid_transfer_state CHECK (
        (status='approved' AND claims_json IS NULL AND document_json IS NULL AND document_sha256 IS NULL) OR
        (status='prepared' AND claims_json IS NOT NULL AND document_json IS NULL AND document_sha256 IS NULL) OR
        (status='issued' AND claims_json IS NOT NULL AND document_json IS NOT NULL AND document_sha256 IS NOT NULL)
    )
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
