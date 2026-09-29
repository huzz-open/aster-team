CREATE TABLE IF NOT EXISTS commercial_paid_fulfillments (
    id VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL PRIMARY KEY,
    operation_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    order_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    payment_id VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    payment_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    customer_id VARCHAR(64) NOT NULL,
    request_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    request_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    environment VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    snapshot_json JSON NOT NULL,
    content_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    approved_by VARCHAR(64) NOT NULL,
    approved_at DATETIME(6) NOT NULL,
    status VARCHAR(16) NOT NULL,
    claims_json JSON NULL,
    document_json JSON NULL,
    document_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NULL,
    UNIQUE KEY uq_paid_fulfillment_operation (operation_id),
    UNIQUE KEY uq_paid_fulfillment_initial_order (order_id),
    KEY ix_paid_fulfillment_request (request_id),
    CONSTRAINT fk_paid_fulfillment_order FOREIGN KEY (order_id) REFERENCES commercial_orders(id),
    CONSTRAINT fk_paid_fulfillment_payment FOREIGN KEY (payment_id) REFERENCES commercial_payment_confirmations(id),
    CONSTRAINT fk_paid_fulfillment_customer FOREIGN KEY (customer_id) REFERENCES customers(id),
    CONSTRAINT fk_paid_fulfillment_actor FOREIGN KEY (approved_by) REFERENCES operators(id),
    CONSTRAINT chk_paid_fulfillment_environment CHECK (environment IN ('local', 'production')),
    CONSTRAINT chk_paid_fulfillment_state CHECK (
        (status='approved' AND claims_json IS NULL AND document_json IS NULL AND document_sha256 IS NULL) OR
        (status='prepared' AND claims_json IS NOT NULL AND document_json IS NULL AND document_sha256 IS NULL) OR
        (status='issued' AND claims_json IS NOT NULL AND document_json IS NOT NULL AND document_sha256 IS NOT NULL)
    )
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

ALTER TABLE operator_permissions
    DROP CONSTRAINT IF EXISTS chk_operator_permissions_registry,
    ADD CONSTRAINT chk_operator_permissions_registry CHECK (permission_code IN (
        'release.read', 'release.build', 'release.download',
        'release.publish.request', 'release.publish.approve', 'release.publish.execute',
        'commercial.plan.read', 'commercial.plan.write',
        'commercial.order.read', 'commercial.order.write', 'commercial.payment.confirm',
        'commercial.fulfillment.read', 'commercial.fulfillment.approve',
        'commercial.distribution.read', 'commercial.distribution.approve', 'commercial.license.issue',
        'commercial.catalog.read', 'commercial.catalog.approve', 'commercial.catalog.export',
        'commercial.publication.read', 'commercial.publication.prepare', 'commercial.publication.accept'
    ));
