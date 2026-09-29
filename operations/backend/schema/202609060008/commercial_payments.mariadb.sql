CREATE TABLE IF NOT EXISTS commercial_payment_confirmations (
    id VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL PRIMARY KEY,
    operation_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    order_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    order_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    snapshot_json JSON NOT NULL,
    content_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    confirmed_by VARCHAR(64) NOT NULL,
    confirmed_at DATETIME(6) NOT NULL,
    UNIQUE KEY uq_commercial_payment_operation (operation_id),
    UNIQUE KEY uq_commercial_payment_order (order_id),
    CONSTRAINT fk_commercial_payment_order FOREIGN KEY (order_id) REFERENCES commercial_orders(id),
    CONSTRAINT fk_commercial_payment_actor FOREIGN KEY (confirmed_by) REFERENCES operators(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

ALTER TABLE operator_permissions
    DROP CONSTRAINT IF EXISTS chk_operator_permissions_registry,
    ADD CONSTRAINT chk_operator_permissions_registry CHECK (permission_code IN (
        'release.read', 'release.build', 'release.download',
        'release.publish.request', 'release.publish.approve', 'release.publish.execute',
        'commercial.plan.read', 'commercial.plan.write',
        'commercial.order.read', 'commercial.order.write', 'commercial.payment.confirm',
        'commercial.distribution.read', 'commercial.distribution.approve', 'commercial.license.issue',
        'commercial.catalog.read', 'commercial.catalog.approve', 'commercial.catalog.export',
        'commercial.publication.read', 'commercial.publication.prepare', 'commercial.publication.accept'
    ));
