CREATE TABLE IF NOT EXISTS `users` (`id` integer PRIMARY KEY AUTOINCREMENT,`username` text,`password` text NOT NULL,`display_name` text,`role` integer DEFAULT 1,`status` integer DEFAULT 1,`email` text,`github_id` text,`discord_id` text,`oidc_id` text,`wechat_id` text,`telegram_id` text,`access_token` char(32),`access_token_created_at` bigint,`quota` integer DEFAULT 0,`used_quota` integer DEFAULT 0,`request_count` integer DEFAULT 0,`group` varchar(64) DEFAULT "default",`aff_code` varchar(32),`aff_count` integer DEFAULT 0,`aff_quota` integer DEFAULT 0,`aff_history` integer DEFAULT 0,`inviter_id` integer,`deleted_at` datetime,`linux_do_id` text,`setting` text,`remark` varchar(255),`stripe_customer` varchar(64),`created_at` integer,`last_login_at` integer DEFAULT 0,`auth_version` bigint NOT NULL DEFAULT 1,CONSTRAINT `uni_users_username` UNIQUE (`username`));
CREATE INDEX IF NOT EXISTS `idx_users_username` ON `users`(`username`);
CREATE INDEX IF NOT EXISTS `idx_users_linux_do_id` ON `users`(`linux_do_id`);
CREATE INDEX IF NOT EXISTS `idx_users_inviter_id` ON `users`(`inviter_id`);
CREATE UNIQUE INDEX IF NOT EXISTS `idx_users_aff_code` ON `users`(`aff_code`);
CREATE INDEX IF NOT EXISTS `idx_users_telegram_id` ON `users`(`telegram_id`);
CREATE INDEX IF NOT EXISTS `idx_users_discord_id` ON `users`(`discord_id`);
CREATE INDEX IF NOT EXISTS `idx_users_git_hub_id` ON `users`(`github_id`);
CREATE INDEX IF NOT EXISTS `idx_users_display_name` ON `users`(`display_name`);
CREATE INDEX IF NOT EXISTS `idx_users_stripe_customer` ON `users`(`stripe_customer`);
CREATE INDEX IF NOT EXISTS `idx_users_deleted_at` ON `users`(`deleted_at`);
CREATE UNIQUE INDEX IF NOT EXISTS `idx_users_access_token` ON `users`(`access_token`);
CREATE INDEX IF NOT EXISTS `idx_users_we_chat_id` ON `users`(`wechat_id`);
CREATE INDEX IF NOT EXISTS `idx_users_oidc_id` ON `users`(`oidc_id`);
CREATE INDEX IF NOT EXISTS `idx_users_email` ON `users`(`email`);

CREATE TABLE IF NOT EXISTS `tokens` (`id` integer PRIMARY KEY AUTOINCREMENT,`user_id` integer,`key` varchar(128),`status` integer DEFAULT 1,`name` text,`created_time` integer,`accessed_time` integer,`expired_time` integer DEFAULT -1,`remain_quota` integer DEFAULT 0,`unlimited_quota` numeric,`model_limits_enabled` numeric,`model_limits` text,`allow_ips` text DEFAULT "",`used_quota` integer DEFAULT 0,`group` text DEFAULT "",`cross_group_retry` numeric,`auto_groups` text,`deleted_at` datetime, `allow_user_quota_query` numeric);
CREATE INDEX IF NOT EXISTS `idx_tokens_deleted_at` ON `tokens`(`deleted_at`);
CREATE INDEX IF NOT EXISTS `idx_tokens_name` ON `tokens`(`name`);
CREATE UNIQUE INDEX IF NOT EXISTS `idx_tokens_key` ON `tokens`(`key`);
CREATE INDEX IF NOT EXISTS `idx_tokens_user_id` ON `tokens`(`user_id`);

CREATE TABLE IF NOT EXISTS `user_sessions` (`sid` varchar(64),`user_id` integer NOT NULL,`version` bigint NOT NULL DEFAULT 1,`user_auth_version` bigint NOT NULL,`status` varchar(16) NOT NULL,`refresh_hash` char(64) NOT NULL,`previous_refresh_hash` varchar(64),`previous_valid_until` bigint NOT NULL DEFAULT 0,`login_method` varchar(32) NOT NULL,`ip` varchar(64),`user_agent` text,`created_at` integer,`last_active_at` bigint NOT NULL,`expires_at` bigint NOT NULL,`revoked_at` bigint NOT NULL DEFAULT 0,`revoked_reason` varchar(64),PRIMARY KEY (`sid`));
CREATE INDEX IF NOT EXISTS `idx_user_sessions_user_status_expiry` ON `user_sessions`(`user_id`,`status`,`expires_at`);
CREATE INDEX IF NOT EXISTS `idx_user_sessions_expires_at` ON `user_sessions`(`expires_at`);
CREATE INDEX IF NOT EXISTS `idx_user_sessions_status_revoked` ON `user_sessions`(`status`,`revoked_at`);
CREATE INDEX IF NOT EXISTS `idx_user_sessions_user_created` ON `user_sessions`(`user_id`,`created_at`);

CREATE TABLE IF NOT EXISTS `options` (`key` text,`value` text,PRIMARY KEY (`key`));

CREATE TABLE IF NOT EXISTS `channels` (`id` integer PRIMARY KEY AUTOINCREMENT,`type` integer DEFAULT 0,`key` text NOT NULL,`open_ai_organization` text,`test_model` text,`status` integer DEFAULT 1,`name` text,`weight` integer DEFAULT 0,`created_time` integer,`test_time` integer,`response_time` integer,`base_url` text DEFAULT "",`other` text,`balance` real,`balance_updated_time` integer,`models` text,`group` varchar(64) DEFAULT "default",`used_quota` integer DEFAULT 0,`model_mapping` text,`status_code_mapping` varchar(1024) DEFAULT "",`priority` integer DEFAULT 0,`auto_ban` integer DEFAULT 1,`other_info` text,`tag` text,`setting` text,`param_override` text,`header_override` text,`remark` varchar(255),`channel_info` json,`settings` text);
CREATE INDEX IF NOT EXISTS `idx_channels_name` ON `channels`(`name`);
CREATE INDEX IF NOT EXISTS `idx_channels_tag` ON `channels`(`tag`);

CREATE TABLE IF NOT EXISTS `abilities` (`group` varchar(64),`model` varchar(255),`channel_id` integer,`enabled` numeric,`priority` integer DEFAULT 0,`weight` integer DEFAULT 0,`tag` text,PRIMARY KEY (`group`,`model`,`channel_id`));
CREATE INDEX IF NOT EXISTS `idx_abilities_tag` ON `abilities`(`tag`);
CREATE INDEX IF NOT EXISTS `idx_abilities_weight` ON `abilities`(`weight`);
CREATE INDEX IF NOT EXISTS `idx_abilities_priority` ON `abilities`(`priority`);
CREATE INDEX IF NOT EXISTS `idx_abilities_channel_id` ON `abilities`(`channel_id`);

CREATE TABLE IF NOT EXISTS `logs` (`id` integer PRIMARY KEY AUTOINCREMENT,`user_id` integer,`created_at` integer,`type` integer,`content` text,`username` text DEFAULT "",`token_name` text DEFAULT "",`model_name` text DEFAULT "",`quota` integer DEFAULT 0,`prompt_tokens` integer DEFAULT 0,`completion_tokens` integer DEFAULT 0,`use_time` integer DEFAULT 0,`is_stream` numeric,`channel_id` integer,`channel_name` text,`token_id` integer DEFAULT 0,`group` text,`ip` text DEFAULT "",`request_id` varchar(64) DEFAULT "",`upstream_request_id` varchar(128) DEFAULT "",`other` text);
CREATE INDEX IF NOT EXISTS `idx_user_id_id` ON `logs`(`user_id`,`id`);
CREATE INDEX IF NOT EXISTS `idx_logs_upstream_request_id` ON `logs`(`upstream_request_id`);
CREATE INDEX IF NOT EXISTS `idx_logs_request_id` ON `logs`(`request_id`);
CREATE INDEX IF NOT EXISTS `idx_logs_ip` ON `logs`(`ip`);
CREATE INDEX IF NOT EXISTS `idx_logs_channel_id` ON `logs`(`channel_id`);
CREATE INDEX IF NOT EXISTS `idx_logs_token_name` ON `logs`(`token_name`);
CREATE INDEX IF NOT EXISTS `idx_created_at_id` ON `logs`(`created_at`,`id`);
CREATE INDEX IF NOT EXISTS `idx_logs_group` ON `logs`(`group`);
CREATE INDEX IF NOT EXISTS `idx_logs_token_id` ON `logs`(`token_id`);
CREATE INDEX IF NOT EXISTS `idx_logs_model_name` ON `logs`(`model_name`);
CREATE INDEX IF NOT EXISTS `index_username_model_name` ON `logs`(`model_name`,`username`);
CREATE INDEX IF NOT EXISTS `idx_logs_username` ON `logs`(`username`);
CREATE INDEX IF NOT EXISTS `idx_created_at_type` ON `logs`(`created_at`,`type`);
CREATE INDEX IF NOT EXISTS `idx_logs_user_id` ON `logs`(`user_id`);

CREATE TABLE IF NOT EXISTS `quota_data` (`id` integer PRIMARY KEY AUTOINCREMENT,`user_id` integer,`username` text DEFAULT "",`model_name` text DEFAULT "",`created_at` integer,`use_group` text DEFAULT "",`token_id` integer DEFAULT 0,`channel_id` integer DEFAULT 0,`node_name` text DEFAULT "",`token_used` integer DEFAULT 0,`count` integer DEFAULT 0,`quota` integer DEFAULT 0, `cache_read_tokens` integer DEFAULT 0, `cache_creation_tokens` integer DEFAULT 0);
CREATE INDEX IF NOT EXISTS `idx_quota_data_token_id` ON `quota_data`(`token_id`);
CREATE INDEX IF NOT EXISTS `idx_quota_data_use_group` ON `quota_data`(`use_group`);
CREATE INDEX IF NOT EXISTS `idx_qdt_created_at` ON `quota_data`(`created_at`);
CREATE INDEX IF NOT EXISTS `idx_qdt_model_user_name` ON `quota_data`(`model_name`,`username`);
CREATE INDEX IF NOT EXISTS `idx_quota_data_user_id` ON `quota_data`(`user_id`);
CREATE INDEX IF NOT EXISTS `idx_quota_data_node_name` ON `quota_data`(`node_name`);
CREATE INDEX IF NOT EXISTS `idx_quota_data_channel_id` ON `quota_data`(`channel_id`);

CREATE TABLE IF NOT EXISTS `token_rate_limits` (`token_id` INTEGER PRIMARY KEY, `rpm` INTEGER NOT NULL DEFAULT 0, `concurrency` INTEGER NOT NULL DEFAULT 0);
