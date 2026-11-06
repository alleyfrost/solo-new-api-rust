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
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import { STATUS_QUERY_KEY } from '@/lib/status-query'

import { getOptions, updateOption } from './api'
import type { OptionRow } from './types'

export const OPTIONS_QUERY_KEY = ['settings', 'options'] as const

export function useOptions() {
  const query = useQuery({
    queryKey: OPTIONS_QUERY_KEY,
    queryFn: async () => {
      const response = await getOptions()
      if (!response.success) throw new Error(response.message ?? '')
      return response.data ?? []
    },
    staleTime: 30 * 1000,
  })

  const values: Record<string, string> = {}
  const rows: OptionRow[] = query.data ?? []
  for (const row of rows) {
    values[row.key] = row.value
  }
  return { ...query, values }
}

/**
 * 站点信息会改变 `/api/status` 的输出，保存后必须一起失效，
 * 否则品牌名与图标要等下一次导航才更新。
 */
export function useSaveOptions() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (changes: Array<[string, string | number | boolean]>) => {
      for (const [key, value] of changes) {
        const response = await updateOption(key, value)
        if (!response.success) {
          throw new Error(response.message ?? 'Failed to save setting')
        }
      }
      return changes.length
    },
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: OPTIONS_QUERY_KEY }),
        queryClient.invalidateQueries({ queryKey: STATUS_QUERY_KEY }),
      ])
    },
  })
}
