ALTER TABLE release_task_artifacts
    ADD COLUMN IF NOT EXISTS platform VARCHAR(40) NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS architecture VARCHAR(40) NOT NULL DEFAULT '';

UPDATE release_task_artifacts a JOIN release_tasks t ON t.id = a.release_task_id
SET a.platform = CASE WHEN a.name = CONCAT('customer-linux-amd64-', t.version) THEN 'linux' ELSE 'windows' END,
    a.architecture = 'amd64',
    a.file_name = CONCAT('aster-team-', t.version, '-', CASE WHEN a.name = CONCAT('customer-linux-amd64-', t.version) THEN 'linux' ELSE 'windows' END, '-amd64.tar.gz')
WHERE a.name IN (CONCAT('customer-linux-amd64-', t.version), CONCAT('customer-windows-amd64-', t.version));

ALTER TABLE release_task_artifacts
    DROP CONSTRAINT IF EXISTS chk_release_task_artifacts_verification,
    ADD CONSTRAINT chk_release_task_artifacts_verification CHECK (
        verification_status IN ('pending', 'queued', 'verified', 'failed', 'unavailable') AND
        runtime_linkage IN ('', 'musl-static', 'msvc') AND
        ((platform = '' AND architecture = '') OR (platform IN ('linux', 'windows') AND architecture = 'amd64')) AND
        (verification_status <> 'verified' OR (
            sha256 IS NOT NULL AND release_manifest_sha256 IS NOT NULL AND release_artifact_id IS NOT NULL AND
            signature_key_id <> '' AND verified_at IS NOT NULL AND
            ((platform = 'linux' AND runtime_linkage = 'musl-static') OR (platform = 'windows' AND runtime_linkage = 'msvc'))
        ))
    );

-- Preserve recovery of verification already in progress. Historical terminal tasks
-- and their unverified Windows packages remain untouched.
UPDATE release_task_artifacts a JOIN release_tasks t ON t.id = a.release_task_id
SET a.verification_status = 'queued'
WHERE t.status = 'verifying' AND t.phase = 'artifact_verification'
    AND a.platform = 'linux' AND a.verification_status = 'pending';

ALTER TABLE release_artifacts
    DROP CONSTRAINT IF EXISTS chk_release_artifact_hash,
    ADD CONSTRAINT chk_release_artifact_hash CHECK (
        sha256 REGEXP '^[0-9a-f]{64}$' AND release_manifest_sha256 REGEXP '^[0-9a-f]{64}$' AND
        architecture = 'amd64' AND size_bytes > 0 AND
        ((platform = 'linux' AND runtime_linkage IN ('musl-static', 'unverified')) OR
         (platform = 'windows' AND runtime_linkage IN ('msvc', 'unverified')))
    );

ALTER TABLE release_tasks
    DROP CONSTRAINT IF EXISTS chk_release_tasks_target,
    DROP COLUMN IF EXISTS platform,
    DROP COLUMN IF EXISTS architecture;
