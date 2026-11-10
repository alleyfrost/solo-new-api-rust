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
import type { TFunction } from 'i18next'
import { z } from 'zod'

import { DEFAULT_GROUP } from '../constants'
import type { ApiKey, ApiKeyFormData } from '../types'

// ============================================================================
// Custom Key
// ============================================================================

// 与后端 `normalize_key`（src/routes/admin/token.rs）同构：长度按剥掉 sk- 之后计，
// `-` 必须禁用——认证侧在第一个连字符处截断，含 `-` 的密钥会认错。
export const CUSTOM_KEY_MIN_LEN = 16
export const CUSTOM_KEY_MAX_LEN = 64
const CUSTOM_KEY_PATTERN = /^[A-Za-z0-9_]+$/

/** 剥掉首尾空白与一层 `sk-` 前缀，返回入库形态的密钥；空串表示自动生成 */
export function normalizeCustomKey(raw?: string): string {
  const trimmed = (raw ?? '').trim()
  return trimmed.startsWith('sk-') ? trimmed.slice(3) : trimmed
}

/** 返回可作为 i18n key 的错误文案，合法（含留空）时返回 null */
export function customKeyErrorMessage(raw?: string): string | null {
  const key = normalizeCustomKey(raw)
  if (key.length === 0) return null
  if (key.length < CUSTOM_KEY_MIN_LEN || key.length > CUSTOM_KEY_MAX_LEN) {
    return 'Token key must be 16-64 characters'
  }
  if (!CUSTOM_KEY_PATTERN.test(key)) {
    return 'Token key may only contain letters, digits or underscore'
  }
  return null
}

// ============================================================================
// Form Schema
// ============================================================================

export function getApiKeyFormSchema(t: TFunction, maxAutoGroups = 5) {
  const autoGroupLimit =
    Number.isInteger(maxAutoGroups) && maxAutoGroups > 0 ? maxAutoGroups : 5

  return z
    .object({
      name: z.string().min(1, t('Please enter a name')),
      key: z.string().optional(),
      expired_time: z.date().nullable(),
      model_limits: z.array(z.string()),
      allow_ips: z.string().optional(),
      group: z.string().optional(),
      auto_groups_mode: z.enum(['inherit', 'custom']),
      auto_groups: z.array(z.string()),
      cross_group_retry: z.boolean().optional(),
      tokenCount: z.number().min(1).optional(),
      rpm: z
        .number()
        .int()
        .min(0, t('Rate limits must not be negative'))
        .optional(),
      concurrency: z
        .number()
        .int()
        .min(0, t('Rate limits must not be negative'))
        .optional(),
    })
    .superRefine((data, ctx) => {
      const keyError = customKeyErrorMessage(data.key)
      if (keyError) {
        ctx.addIssue({ code: 'custom', path: ['key'], message: t(keyError) })
      } else if (
        normalizeCustomKey(data.key).length > 0 &&
        (data.tokenCount ?? 1) > 1
      ) {
        // 批量创建会复用同一份密钥，第二个必然撞唯一索引
        ctx.addIssue({
          code: 'custom',
          path: ['tokenCount'],
          message: t('A custom key can only be used for one key at a time'),
        })
      }

      if (data.group === 'auto') {
        if (
          data.auto_groups_mode === 'custom' &&
          data.auto_groups.length === 0
        ) {
          ctx.addIssue({
            code: 'custom',
            path: ['auto_groups'],
            message: t(
              'Select at least one Auto group or restore global Auto.'
            ),
          })
        }

        if (data.auto_groups.length > autoGroupLimit) {
          ctx.addIssue({
            code: 'custom',
            path: ['auto_groups'],
            message: t('Select at most {{max}} Auto groups', {
              max: autoGroupLimit,
            }),
          })
        }

        if (new Set(data.auto_groups).size !== data.auto_groups.length) {
          ctx.addIssue({
            code: 'custom',
            path: ['auto_groups'],
            message: t('Auto groups must not contain duplicates'),
          })
        }
      }
    })
}

export type ApiKeyFormValues = z.infer<ReturnType<typeof getApiKeyFormSchema>>

// ============================================================================
// Form Defaults
// ============================================================================

export const API_KEY_FORM_DEFAULT_VALUES: ApiKeyFormValues = {
  name: '',
  key: '',
  expired_time: null,
  model_limits: [],
  allow_ips: '',
  group: DEFAULT_GROUP,
  auto_groups_mode: 'inherit',
  auto_groups: [],
  cross_group_retry: true,
  tokenCount: 1,
  rpm: 0,
  concurrency: 0,
}

export function getApiKeyFormDefaultValues(
  defaultUseAutoGroup: boolean
): ApiKeyFormValues {
  return {
    ...API_KEY_FORM_DEFAULT_VALUES,
    group: defaultUseAutoGroup ? 'auto' : DEFAULT_GROUP,
    auto_groups_mode: 'inherit',
    auto_groups: [],
    cross_group_retry: defaultUseAutoGroup,
  }
}

// ============================================================================
// Form Data Transformation
// ============================================================================

/**
 * Transform form data to API payload
 */
export function transformFormDataToPayload(
  data: ApiKeyFormValues
): ApiKeyFormData {
  // 只记用量不记金额：额度类字段不再提交，由服务端取默认值（新建 unlimited）或沿用原值（更新）。
  const key = normalizeCustomKey(data.key)
  return {
    name: data.name,
    // 留空时不带字段，由服务端随机生成；更新分支不会走到这里
    ...(key.length > 0 ? { key } : {}),
    expired_time: data.expired_time
      ? Math.floor(data.expired_time.getTime() / 1000)
      : -1,
    model_limits_enabled: data.model_limits.length > 0,
    model_limits: data.model_limits.join(','),
    allow_ips: data.allow_ips || '',
    group: data.group || '',
    auto_groups:
      data.group === 'auto' && data.auto_groups_mode === 'custom'
        ? data.auto_groups
        : [],
    cross_group_retry: data.group === 'auto' ? !!data.cross_group_retry : false,
    // 限流配置始终随表单提交：0 表示不限；更新分支显式 0 即清除配置。
    rpm: data.rpm ?? 0,
    concurrency: data.concurrency ?? 0,
  }
}

/**
 * Transform API key data to form defaults
 */
export function transformApiKeyToFormDefaults(
  apiKey: ApiKey,
  availableAutoGroups: string[] = [],
  maxAutoGroups = 5
): ApiKeyFormValues {
  const availableSet = new Set(availableAutoGroups)
  const storedAutoGroups = apiKey.auto_groups ?? []
  const autoGroups = storedAutoGroups
    .filter((group) => availableSet.has(group))
    .slice(0, Math.max(0, maxAutoGroups))
  const autoGroupsMode = storedAutoGroups.length > 0 ? 'custom' : 'inherit'

  return {
    name: apiKey.name,
    // 密钥只在创建时可指定，编辑态不回填也不提交
    key: '',
    expired_time:
      apiKey.expired_time > 0 ? new Date(apiKey.expired_time * 1000) : null,
    model_limits: apiKey.model_limits
      ? apiKey.model_limits.split(',').filter(Boolean)
      : [],
    allow_ips: apiKey.allow_ips || '',
    group: apiKey.group || DEFAULT_GROUP,
    auto_groups_mode: autoGroupsMode,
    auto_groups: autoGroups,
    cross_group_retry: !!apiKey.cross_group_retry,
    tokenCount: 1,
    rpm: apiKey.rpm ?? 0,
    concurrency: apiKey.concurrency ?? 0,
  }
}
