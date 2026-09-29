CREATE TABLE IF NOT EXISTS trial_leads (
  id TEXT PRIMARY KEY,
  created_at TEXT NOT NULL,
  contact TEXT NOT NULL,
  company TEXT NOT NULL DEFAULT '',
  team_size INTEGER NOT NULL,
  active_users INTEGER NOT NULL,
  evidence TEXT NOT NULL CHECK (evidence IN ('tokens', 'time')),
  weekly_tokens_100m REAL,
  daily_time TEXT CHECK (daily_time IS NULL OR daily_time IN ('under1h', '1h-4h', '4h-8h', 'over8h')),
  usage_level INTEGER NOT NULL CHECK (usage_level BETWEEN 0 AND 2),
  recommended_pro_accounts INTEGER NOT NULL,
  locale TEXT NOT NULL CHECK (locale IN ('zh', 'en')),
  notification_status TEXT NOT NULL CHECK (notification_status IN ('pending', 'sent', 'failed', 'not_configured')),
  notification_error TEXT,
  turnstile_hostname TEXT NOT NULL,
  client_fingerprint TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_trial_leads_created_at
  ON trial_leads (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_trial_leads_notification_status
  ON trial_leads (notification_status, created_at DESC);

CREATE TABLE IF NOT EXISTS trial_rate_limits (
  rate_key TEXT PRIMARY KEY,
  window_started_at INTEGER NOT NULL,
  count INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_trial_rate_limits_expires_at
  ON trial_rate_limits (expires_at);
