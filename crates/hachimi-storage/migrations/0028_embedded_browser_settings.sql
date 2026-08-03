PRAGMA foreign_keys = ON;

ALTER TABLE browser_profiles ADD COLUMN download_directory TEXT;
ALTER TABLE browser_profiles ADD COLUMN ask_where_to_save_downloads INTEGER NOT NULL DEFAULT 0 CHECK(ask_where_to_save_downloads IN (0, 1));
ALTER TABLE browser_profiles ADD COLUMN full_cdp_access INTEGER NOT NULL DEFAULT 0 CHECK(full_cdp_access IN (0, 1));
ALTER TABLE browser_profiles ADD COLUMN settings_revision INTEGER NOT NULL DEFAULT 1 CHECK(settings_revision > 0);
