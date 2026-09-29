CREATE TABLE IF NOT EXISTS commercial_catalog_publications (
    id VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL PRIMARY KEY,
    operation_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    environment VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    catalog_revision VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    snapshot_json JSON NOT NULL,
    content_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    created_by VARCHAR(64) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    status VARCHAR(16) NOT NULL,
    evidence_json JSON NULL,
    accepted_by VARCHAR(64) NULL,
    accepted_at DATETIME(6) NULL,
    UNIQUE KEY uq_commercial_publication_operation (operation_id),
    CONSTRAINT fk_commercial_publication_catalog FOREIGN KEY (catalog_revision) REFERENCES commercial_catalog_approvals(id),
    CONSTRAINT fk_commercial_publication_creator FOREIGN KEY (created_by) REFERENCES operators(id),
    CONSTRAINT fk_commercial_publication_acceptor FOREIGN KEY (accepted_by) REFERENCES operators(id),
    CONSTRAINT chk_commercial_publication_environment CHECK (environment IN ('local', 'production')),
    CONSTRAINT chk_commercial_publication_state CHECK (
        (status='prepared' AND evidence_json IS NULL AND accepted_by IS NULL AND accepted_at IS NULL) OR
        (status='accepted' AND evidence_json IS NOT NULL AND accepted_by IS NOT NULL AND accepted_at IS NOT NULL)
    )
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS commercial_publication_heads (
    environment VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL PRIMARY KEY,
    active_id VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NULL,
    CONSTRAINT fk_commercial_publication_head FOREIGN KEY (active_id) REFERENCES commercial_catalog_publications(id),
    CONSTRAINT chk_commercial_publication_head_environment CHECK (environment IN ('local', 'production'))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

INSERT INTO commercial_publication_heads(environment) VALUES ('local'),('production')
    ON DUPLICATE KEY UPDATE environment=VALUES(environment);

ALTER TABLE operator_permissions
    DROP CONSTRAINT IF EXISTS chk_operator_permissions_registry,
    ADD CONSTRAINT chk_operator_permissions_registry CHECK (permission_code IN (
        'release.read', 'release.build', 'release.download',
        'release.publish.request', 'release.publish.approve', 'release.publish.execute',
        'commercial.plan.read', 'commercial.plan.write',
        'commercial.order.read', 'commercial.order.write',
        'commercial.distribution.read', 'commercial.distribution.approve', 'commercial.license.issue',
        'commercial.catalog.read', 'commercial.catalog.approve', 'commercial.catalog.export',
        'commercial.publication.read', 'commercial.publication.prepare', 'commercial.publication.accept'
    ));
