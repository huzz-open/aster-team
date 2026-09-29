ALTER TABLE operator_permissions
    DROP CONSTRAINT IF EXISTS chk_operator_permissions_release,
    ADD CONSTRAINT chk_operator_permissions_release CHECK (permission_code IN (
        'release.read', 'release.build', 'release.download',
        'release.publish.request', 'release.publish.approve', 'release.publish.execute',
        'release.environment.write', 'release.environment.upgrade'
    ));

CREATE TABLE IF NOT EXISTS upgrade_environments (
    id VARCHAR(80) NOT NULL PRIMARY KEY,
    installation_id VARCHAR(128) NOT NULL UNIQUE,
    metadata_json JSON NOT NULL,
    credentials_ciphertext TEXT NOT NULL,
    created_at DATETIME(6) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS environment_upgrades (
    id VARCHAR(80) NOT NULL PRIMARY KEY,
    environment_id VARCHAR(80) NOT NULL,
    active_environment_id VARCHAR(80) NULL UNIQUE,
    task_json JSON NOT NULL,
    lease_owner VARCHAR(80) NOT NULL DEFAULT '',
    lease_until DATETIME(6) NULL,
    created_at DATETIME(6) NOT NULL,
    CONSTRAINT fk_upgrade_environment FOREIGN KEY (environment_id) REFERENCES upgrade_environments(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS upgrade_probe_samples (
    sequence BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
    task_id VARCHAR(80) NOT NULL,
    sample_json JSON NOT NULL,
    INDEX idx_upgrade_sample_task (task_id, sequence),
    CONSTRAINT fk_upgrade_sample_task FOREIGN KEY (task_id) REFERENCES environment_upgrades(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

INSERT IGNORE INTO operator_permissions (operator_id, permission_code, granted_at)
SELECT operator_id, 'release.environment.write', UTC_TIMESTAMP(6) FROM operator_permissions WHERE permission_code='release.build';

INSERT IGNORE INTO operator_permissions (operator_id, permission_code, granted_at)
SELECT operator_id, 'release.environment.upgrade', UTC_TIMESTAMP(6) FROM operator_permissions WHERE permission_code='release.build';
