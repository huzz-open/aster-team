ALTER TABLE release_tasks
    ADD COLUMN IF NOT EXISTS free_distribution_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NULL AFTER source_commit_sha,
    ADD COLUMN IF NOT EXISTS free_license_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NULL AFTER free_distribution_id;

ALTER TABLE release_tasks
    DROP CONSTRAINT IF EXISTS fk_release_tasks_free_distribution,
    DROP CONSTRAINT IF EXISTS chk_release_tasks_free_license;

ALTER TABLE release_tasks
    MODIFY COLUMN free_distribution_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NULL,
    MODIFY COLUMN free_license_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NULL;

ALTER TABLE release_tasks
    ADD CONSTRAINT fk_release_tasks_free_distribution
        FOREIGN KEY (free_distribution_id) REFERENCES commercial_free_distributions(id),
    ADD CONSTRAINT chk_release_tasks_free_license CHECK (
        (free_distribution_id IS NULL AND free_license_sha256 IS NULL) OR
        (free_distribution_id IS NOT NULL AND free_license_sha256 IS NOT NULL AND
         free_license_sha256 REGEXP '^[0-9a-f]{64}$')
    );
