/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
/**
 * Type definitions for usage logs
 */
import type { UsageLog } from './data/schema'
// ============================================================================
// Filter Types
// ============================================================================

/**
 * Common logs filters
 */
export interface CommonLogFilters {
  startTime?: Date
  endTime?: Date
  channel?: string
  model?: string
  token?: string
  group?: string
  username?: string
  requestId?: string
  upstreamRequestId?: string
}

// ============================================================================
// Common Logs Additional Types
// ============================================================================

/**
 * Parsed data from the 'other' field in usage logs.
 *
 * This product records usage only, never money. Any billing / price / ratio /
 * subscription fields present in historical JSON payloads are intentionally
 * not modeled or displayed.
 */
export interface LogOtherData {
  admin_info?: {
    is_multi_key?: boolean
    multi_key_index?: number
    use_channel?: number[]
    // Top-up audit fields (type=1, admin only)
    payment_method?: string
    callback_payment_method?: string
    caller_ip?: string
    server_ip?: string
    version?: string
    node_name?: string
    // Operator identity for audit logs (type=3, admin only)
    admin_username?: string
    admin_id?: number | string
    admin_role?: number
    auth_method?: 'session' | 'access_token' | string
    // Reject / intercept reason (admin only)
    reject_reason?: string
  }
  // Structured log content: English source keys plus params, rendered in the
  // viewer's language. `content` keeps the English text for older clients.
  content_parts?: {
    key: string
    params?: Record<string, string | number | boolean>
  }[]
  // Language-independent operation descriptor (audit/login logs).
  // Frontend renders localized content from action + params via i18n templates.
  op?: {
    action?: string
    params?: Record<string, string | number | boolean | string[]>
  }
  // Operation audit details written by the admin-audit fallback in authHelper (type=3, admin only)
  audit_info?: {
    method?: string
    route?: string
    path?: string
    status?: number
    success?: boolean
    params?: Record<string, string>
  }
  // Login audit fields (type=7); visible to the log owner
  login_method?: string
  user_agent?: string
  request_path?: string
  request_conversion?: string[]
  ws?: boolean
  audio?: boolean
  audio_input?: number
  audio_output?: number
  text_input?: number
  text_output?: number
  cache_tokens?: number
  image_cache_tokens?: number
  billing_tokens?: Record<string, number>
  cache_creation_tokens?: number
  cache_creation_tokens_5m?: number
  cache_creation_tokens_1h?: number
  cache_creation_tokens_12h?: number
  is_model_mapped?: boolean
  upstream_model_name?: string
  // Diagnostic only. Whether the names disagree is derived in the UI via
  // isResponseModelMismatch so old rows follow the current comparison rule.
  response_model?: {
    requested_model: string
    upstream_model: string
    returned_model: string
  }
  frt?: number
  usage_facts?: Record<string, string | number>
  reasoning_effort?: string
  web_search?: boolean
  web_search_call_count?: number
  file_search?: boolean
  file_search_call_count?: number
  is_system_prompt_overwritten?: boolean
  po?: string[]
  group?: string
  stream_status?: {
    status?: string
    end_reason?: string
    error_count?: number
    end_error?: string
    errors?: string[]
  }
  // Task-related fields (for refund logs, type=6)
  is_task?: boolean
  // The submitting request returned the task result itself (an immediate
  // result, or an OpenAI Images request the gateway waited on).
  task_sync?: boolean
  // The inline result was not persisted, so no artifact can be retrieved.
  result_discarded?: boolean
  task_id?: string
  reason?: string
}

/**
 * Log statistics data
 */
export interface LogStatistics {
  rpm: number
  tpm: number
}

// ============================================================================
// Common Log Types
// ============================================================================

export interface GetLogsParams {
  p?: number
  page_size?: number
  type?: number
  username?: string
  token_name?: string
  model_name?: string
  start_timestamp?: number
  end_timestamp?: number
  channel?: number
  group?: string
  request_id?: string
  upstream_request_id?: string
}

export interface GetLogsResponse {
  success: boolean
  message?: string
  data?: {
    items: UsageLog[]
    total: number
    page: number
    page_size: number
  }
}

export interface GetLogStatsParams {
  type?: number
  username?: string
  token_name?: string
  model_name?: string
  start_timestamp?: number
  end_timestamp?: number
  channel?: number
  group?: string
  request_id?: string
  upstream_request_id?: string
}

export interface GetLogStatsResponse {
  success: boolean
  message?: string
  data?: LogStatistics
}

// ============================================================================
// User Info Types
// ============================================================================

export interface UserInfo {
  id: number
  username: string
  display_name?: string
  request_count: number
  group?: string
  remark?: string
}
