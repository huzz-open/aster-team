CREATE TABLE IF NOT EXISTS commercial_plan_draft_heads (
    id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL PRIMARY KEY,
    current_revision INT UNSIGNED NOT NULL,
    updated_at DATETIME(6) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS commercial_plan_draft_revisions (
    draft_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    revision_no INT UNSIGNED NOT NULL,
    snapshot_json JSON NOT NULL,
    content_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    operation_id VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    created_by VARCHAR(64) NOT NULL,
    created_at DATETIME(6) NOT NULL,
    PRIMARY KEY (draft_id, revision_no),
    UNIQUE KEY uq_commercial_draft_operation (operation_id),
    CONSTRAINT fk_commercial_draft_head FOREIGN KEY (draft_id) REFERENCES commercial_plan_draft_heads(id),
    CONSTRAINT fk_commercial_draft_operator FOREIGN KEY (created_by) REFERENCES operators(id),
    CONSTRAINT chk_commercial_draft_revision CHECK (revision_no > 0)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
