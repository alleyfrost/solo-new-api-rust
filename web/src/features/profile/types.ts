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
import type { LoginSession } from '@/stores/auth-store'

// ============================================================================
// Profile Type Definitions
// ============================================================================

export interface ApiResponse<T = unknown> {
  success: boolean
  message?: string
  data?: T
}

/**
 * What `GET /api/user/self` returns. Usage figures come from the log table:
 * this deployment records usage only, never money, so there is no balance.
 */
export interface UserProfile {
  id: number
  username: string
  display_name: string
  role: number
  status: number
  group: string
  email?: string
  has_password?: boolean
  setting?: string
  created_time: number
  request_count: number
  prompt_tokens: number
  completion_tokens: number
}

export interface UpdateProfileRequest {
  display_name?: string
  language?: string
  password?: string
  original_password?: string
}

/** Keys of the `users.setting` JSON blob that this UI owns. */
export interface UserSettings {
  language?: string
}

export interface PasswordChangeResult {
  access_token: string
  token_type: string
  access_expires_at: number
  has_password: boolean
  session: LoginSession
}
