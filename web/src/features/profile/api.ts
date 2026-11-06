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
import { api } from '@/lib/api'
import type { LoginSession } from '@/stores/auth-store'

import type {
  ApiResponse,
  PasswordChangeResult,
  UpdateProfileRequest,
  UserProfile,
} from './types'

// ============================================================================
// User Profile APIs
// ============================================================================

export async function getUserProfile(): Promise<ApiResponse<UserProfile>> {
  const res = await api.get('/api/user/self')
  return res.data
}

export async function updateUserProfile(
  data: UpdateProfileRequest
): Promise<ApiResponse> {
  const res = await api.put('/api/user/self', data, {
    acceptAuthRotation: Boolean(data.password),
  })
  return res.data
}

/**
 * Rotating the password invalidates every other session, so the server returns
 * a fresh access token for the current one; the http client swaps it in when
 * `acceptAuthRotation` is set.
 */
export async function changePassword(
  data: { password: string; original_password?: string },
  signal?: AbortSignal
): Promise<ApiResponse<PasswordChangeResult>> {
  const res = await api.put('/api/user/self', data, {
    acceptAuthRotation: true,
    signal,
  })
  return res.data
}

export async function updateUserLanguage(
  language: string
): Promise<ApiResponse> {
  const res = await api.put('/api/user/self', { language })
  return res.data
}

// ============================================================================
// Login Sessions
// ============================================================================

export async function getLoginSessions(): Promise<ApiResponse<LoginSession[]>> {
  const res = await api.get('/api/user/sessions')
  return res.data
}

export async function revokeLoginSession(sid: string): Promise<ApiResponse> {
  const res = await api.delete(`/api/user/sessions/${encodeURIComponent(sid)}`)
  return res.data
}

export async function revokeOtherLoginSessions(): Promise<ApiResponse> {
  const res = await api.post('/api/user/sessions/revoke-others')
  return res.data
}
