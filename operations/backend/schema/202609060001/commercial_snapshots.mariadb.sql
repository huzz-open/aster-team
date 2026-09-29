CREATE TABLE IF NOT EXISTS commercial_plan_heads (
    id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL PRIMARY KEY,
    code VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    current_version INT UNSIGNED NOT NULL,
    created_at DATETIME(6) NOT NULL,
    updated_at DATETIME(6) NOT NULL,
    UNIQUE KEY uq_commercial_plan_code (code)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS commercial_plan_versions (
    plan_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    version_no INT UNSIGNED NOT NULL,
    operation_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    snapshot_json JSON NOT NULL,
    content_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    created_by VARCHAR(64) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    PRIMARY KEY (plan_id, version_no),
    UNIQUE KEY uq_commercial_plan_operation (operation_id),
    CONSTRAINT fk_commercial_plan_head FOREIGN KEY (plan_id) REFERENCES commercial_plan_heads(id),
    CONSTRAINT fk_commercial_plan_operator FOREIGN KEY (created_by) REFERENCES operators(id),
    CONSTRAINT chk_commercial_plan_version CHECK (version_no > 0)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS commercial_orders (
    id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL PRIMARY KEY,
    operation_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    customer_id VARCHAR(64) NOT NULL,
    plan_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    plan_version INT UNSIGNED NOT NULL,
    snapshot_json JSON NOT NULL,
    content_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    status VARCHAR(40) NOT NULL,
    created_by VARCHAR(64) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    updated_at DATETIME(6) NOT NULL,
    UNIQUE KEY uq_commercial_order_operation (operation_id),
    KEY idx_commercial_order_customer (customer_id, created_at),
    CONSTRAINT fk_commercial_order_customer FOREIGN KEY (customer_id) REFERENCES customers(id),
    CONSTRAINT fk_commercial_order_plan FOREIGN KEY (plan_id, plan_version) REFERENCES commercial_plan_versions(plan_id, version_no),
    CONSTRAINT fk_commercial_order_operator FOREIGN KEY (created_by) REFERENCES operators(id),
    CONSTRAINT chk_commercial_order_status CHECK (status IN ('pending_payment', 'fulfillment_pending', 'fulfilled', 'cancelled', 'refunded'))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
