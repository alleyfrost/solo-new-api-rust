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

import type {
  ApiResponse,
  LogCleanupResult,
  MigrationResult,
  OptionRow,
} from './types'

export async function getOptions(): Promise<ApiResponse<OptionRow[]>> {
  const res = await api.get('/api/option/')
  return res.data
}

export async function updateOption(
  key: string,
  value: string | number | boolean
): Promise<ApiResponse> {
  const res = await api.put('/api/option/', { key, value })
  return res.data
}

export async function migrateFromNewApi(
  backup: boolean
): Promise<ApiResponse<MigrationResult>> {
  const res = await api.post('/api/migrate/new-api', { backup })
  return res.data
}

/**
 * 一次调用只删一批；`has_more` 为真时调用方继续推进，直到删空或主动中断。
 */
export async function cleanupLogsBefore(
  before: number
): Promise<ApiResponse<LogCleanupResult>> {
  const res = await api.delete('/api/log/cleanup', { params: { before } })
  return res.data
}
