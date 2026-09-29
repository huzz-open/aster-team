CREATE TABLE IF NOT EXISTS commercial_catalog_approvals (
    id VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL PRIMARY KEY,
    operation_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    environment VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    snapshot_json JSON NOT NULL,
    content_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    public_json JSON NOT NULL,
    public_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    approved_by VARCHAR(64) NOT NULL,
    approved_at DATETIME(6) NOT NULL,
    status VARCHAR(16) NOT NULL,
    UNIQUE KEY uq_commercial_catalog_operation (operation_id),
    CONSTRAINT fk_commercial_catalog_operator FOREIGN KEY (approved_by) REFERENCES operators(id),
    CONSTRAINT chk_commercial_catalog_environment CHECK (environment IN ('local', 'production')),
    CONSTRAINT chk_commercial_catalog_status CHECK (status IN ('approved', 'exported'))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

ALTER TABLE operator_permissions
    DROP CONSTRAINT IF EXISTS chk_operator_permissions_registry,
    ADD CONSTRAINT chk_operator_permissions_registry CHECK (permission_code IN (
        'release.read', 'release.build', 'release.download',
        'release.publish.request', 'release.publish.approve', 'release.publish.execute',
        'commercial.plan.read', 'commercial.plan.write',
        'commercial.order.read', 'commercial.order.write',
        'commercial.distribution.read', 'commercial.distribution.approve', 'commercial.license.issue',
        'commercial.catalog.read', 'commercial.catalog.approve', 'commercial.catalog.export'
    ));
