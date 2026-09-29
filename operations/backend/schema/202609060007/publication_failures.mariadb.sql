CREATE TABLE IF NOT EXISTS commercial_publication_failures (
    id VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL PRIMARY KEY,
    publication_id VARCHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    event_json JSON NOT NULL,
    content_sha256 CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
    created_at DATETIME(6) NOT NULL,
    KEY ix_commercial_publication_failure_history (publication_id, created_at, id),
    CONSTRAINT fk_commercial_publication_failure FOREIGN KEY (publication_id) REFERENCES commercial_catalog_publications(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
