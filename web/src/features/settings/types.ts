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
export interface ApiResponse<T = unknown> {
  success: boolean
  message?: string
  data?: T
}

export interface OptionRow {
  key: string
  value: string
}

export interface LogCleanupResult {
  deleted: number
  remaining: number
  has_more: boolean
}

export const OPTION_KEYS = {
  systemName: 'SystemName',
  logo: 'Logo',
  notice: 'Notice',
  serverAddress: 'ServerAddress',
  logConsume: 'LogConsumeEnabled',
  logExport: 'log_setting.user_export_enabled',
  logRangeDays: 'log_setting.max_range_days',
} as const

export const SYSTEM_NAME_MAX_LENGTH = 64
export const LOGO_MAX_LENGTH = 512
export const NOTICE_MAX_LENGTH = 2000
export const ADDRESS_MAX_LENGTH = 256
export const MIN_RANGE_DAYS = 1
export const MAX_RANGE_DAYS = 3650
