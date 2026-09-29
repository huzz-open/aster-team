CREATE TABLE IF NOT EXISTS commercial_free_distributions (
    id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL PRIMARY KEY,
    operation_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    plan_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    plan_version INT UNSIGNED NOT NULL,
    snapshot_json JSON NOT NULL,
    content_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    approved_by VARCHAR(64) NOT NULL,
    approved_at DATETIME(6) NOT NULL,
    status VARCHAR(40) NOT NULL,
    claims_json JSON NULL,
    document_json JSON NULL,
    document_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NULL,
    UNIQUE KEY uq_commercial_distribution_operation (operation_id),
    CONSTRAINT fk_commercial_distribution_plan FOREIGN KEY (plan_id, plan_version) REFERENCES commercial_plan_versions(plan_id, version_no),
    CONSTRAINT fk_commercial_distribution_operator FOREIGN KEY (approved_by) REFERENCES operators(id),
    CONSTRAINT chk_commercial_distribution_state CHECK (
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
        'commercial.order.read', 'commercial.order.write',
        'commercial.distribution.read', 'commercial.distribution.approve', 'commercial.license.issue'
    ));
