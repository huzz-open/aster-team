CREATE TABLE IF NOT EXISTS product_inquiries (
  id TEXT PRIMARY KEY,
  created_at TEXT NOT NULL,
  request_sha256 TEXT NOT NULL,
  contact TEXT NOT NULL,
  message TEXT NOT NULL,
  locale TEXT NOT NULL CHECK (locale IN ('zh', 'en')),
  catalog_reference_json TEXT,
  reference_status TEXT NOT NULL CHECK (reference_status IN ('none', 'unverified')),
  notification_status TEXT NOT NULL CHECK (notification_status IN ('pending', 'sent', 'failed', 'not_configured')),
  notification_error TEXT,
  turnstile_hostname TEXT NOT NULL,
  client_fingerprint TEXT NOT NULL,
  CHECK ((reference_status = 'none' AND catalog_reference_json IS NULL)
    OR (reference_status = 'unverified' AND catalog_reference_json IS NOT NULL))
);

CREATE INDEX IF NOT EXISTS idx_product_inquiries_created_at ON product_inquiries (created_at DESC);
CREATE INDEX IF NOT EXISTS idx_product_inquiries_notification_status ON product_inquiries (notification_status, created_at DESC);
