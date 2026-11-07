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
import type { AuthBundle } from '@/stores/auth-store'

// ============================================================================
// API Payloads
// ============================================================================

export interface LoginPayload {
  username: string
  password: string
}

// ============================================================================
// API Responses
// ============================================================================

export interface LoginResponse {
  success: boolean
  message: string
  data?: AuthBundle
}

export interface ApiResponse<T = unknown> {
  success: boolean
  message: string
  data?: T
}

// ============================================================================
// System Status
// ============================================================================

export interface SystemStatus {
  success?: boolean
  message?: string
  data?: {
    version?: string
    system_name?: string
    logo?: string
    display_in_currency?: boolean
    display_token_stat_enabled?: boolean
    quota_per_unit?: number
    quota_display_type?: string
    usd_exchange_rate?: number
    custom_currency_symbol?: string
    custom_currency_exchange_rate?: number
    demo_site_enabled?: boolean
    user_agreement_enabled?: boolean
    privacy_policy_enabled?: boolean
    [key: string]: unknown
  }
  // Allow direct access to common properties
  version?: string
  system_name?: string
  logo?: string
  image_models?: string[]
  display_in_currency?: boolean
  display_token_stat_enabled?: boolean
  quota_per_unit?: number
  quota_display_type?: string
  usd_exchange_rate?: number
  custom_currency_symbol?: string
  custom_currency_exchange_rate?: number
  demo_site_enabled?: boolean
  user_agreement_enabled?: boolean
  privacy_policy_enabled?: boolean
  log_export_enabled?: boolean
  log_max_range_days?: number
  quota_remind_enabled?: boolean
  /** Site default low balance reminder threshold, in quota units */
  quota_remind_threshold?: number
  [key: string]: unknown
}

// ============================================================================
// Form Props
// ============================================================================

export interface AuthFormProps extends React.HTMLAttributes<HTMLFormElement> {
  redirectTo?: string
}
