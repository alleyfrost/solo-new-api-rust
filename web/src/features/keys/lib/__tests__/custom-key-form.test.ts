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
import { describe, expect, test } from 'vitest'

import type { ApiKey } from '../../types'
import {
  customKeyErrorMessage,
  getApiKeyFormDefaultValues,
  getApiKeyFormSchema,
  normalizeCustomKey,
  transformApiKeyToFormDefaults,
  transformFormDataToPayload,
  type ApiKeyFormValues,
} from '../api-key-form'

const t = ((key: string) => key) as TFunction
const schema = getApiKeyFormSchema(t)

const VALID_KEY = 'my_custom_key_2026'

function formValues(
  overrides: Partial<ApiKeyFormValues> = {}
): ApiKeyFormValues {
  return {
    ...getApiKeyFormDefaultValues(false),
    name: 'cli',
    ...overrides,
  }
}

function keyIssues(values: Partial<ApiKeyFormValues>): string[] {
  const result = schema.safeParse(formValues(values))
  if (result.success) return []
  return result.error.issues.map((issue) => issue.message)
}

describe('custom key normalization', () => {
  test('strips whitespace and one leading sk- prefix', () => {
    expect(normalizeCustomKey(`  sk-${VALID_KEY}  `)).toBe(VALID_KEY)
    expect(normalizeCustomKey(VALID_KEY)).toBe(VALID_KEY)
    expect(normalizeCustomKey('sk-')).toBe('')
    expect(normalizeCustomKey(undefined)).toBe('')
  })

  test('keeps a second sk- segment as part of the key', () => {
    expect(normalizeCustomKey('sk-sk_abcdefghij123456')).toBe(
      'sk_abcdefghij123456'
    )
  })
})

describe('custom key validation', () => {
  test('accepts blank input so the server generates the key', () => {
    expect(customKeyErrorMessage('')).toBeNull()
    expect(customKeyErrorMessage('   ')).toBeNull()
    expect(customKeyErrorMessage(VALID_KEY)).toBeNull()
  })

  test('rejects lengths outside 16-64 after the prefix is removed', () => {
    expect(customKeyErrorMessage('abc')).toBe(
      'Token key must be 16-64 characters'
    )
    expect(customKeyErrorMessage('a'.repeat(65))).toBe(
      'Token key must be 16-64 characters'
    )
    expect(customKeyErrorMessage(`sk-${'a'.repeat(65)}`)).toBe(
      'Token key must be 16-64 characters'
    )
    expect(customKeyErrorMessage('a'.repeat(16))).toBeNull()
    expect(customKeyErrorMessage('a'.repeat(64))).toBeNull()
    expect(customKeyErrorMessage(`sk-${'a'.repeat(64)}`)).toBeNull()
  })

  test('rejects hyphens because authentication truncates at the first one', () => {
    expect(customKeyErrorMessage('my-key-with-hyphens')).toBe(
      'Token key may only contain letters, digits or underscore'
    )
    expect(customKeyErrorMessage('密钥_not_ascii_key')).toBe(
      'Token key may only contain letters, digits or underscore'
    )
  })

  test('surfaces the same wording the backend uses', () => {
    expect(keyIssues({ key: 'short' })).toContain(
      'Token key must be 16-64 characters'
    )
    expect(keyIssues({ key: 'my-hyphenated-key-1' })).toContain(
      'Token key may only contain letters, digits or underscore'
    )
    expect(keyIssues({ key: VALID_KEY })).toHaveLength(0)
  })

  test('blocks batching a custom key across several tokens', () => {
    expect(keyIssues({ key: VALID_KEY, tokenCount: 2 })).toEqual([
      'A custom key can only be used for one key at a time',
    ])
    expect(keyIssues({ key: '', tokenCount: 2 })).toHaveLength(0)
  })
})

describe('custom key payload', () => {
  test('submits the normalized key only when provided', () => {
    const withKey = transformFormDataToPayload(
      formValues({ key: `sk-${VALID_KEY}` })
    )
    expect(withKey.name).toBe('cli')
    expect(withKey.key).toBe(VALID_KEY)

    const payload = transformFormDataToPayload(formValues({ key: '  ' }))
    expect('key' in payload).toBe(false)
  })

  test('editing never carries a key back into the form', () => {
    const defaults = transformApiKeyToFormDefaults({
      id: 1,
      name: 'cli',
      key: VALID_KEY,
      status: 1,
      remain_quota: 0,
      used_quota: 0,
      request_count: 0,
      used_tokens: 0,
      cache_read_tokens: 0,
      unlimited_quota: true,
      expired_time: -1,
      created_time: 1,
      accessed_time: 0,
      group: 'default',
      auto_groups: null,
      cross_group_retry: false,
      model_limits_enabled: false,
      model_limits: '',
      allow_ips: '',
      allow_user_quota_query: true,
      rpm: 0,
      concurrency: 0,
    })

    expect(defaults.key).toBe('')
    expect('key' in transformFormDataToPayload(defaults)).toBe(false)
  })
})

describe('per-token rate limits', () => {
  test('defaults to unlimited (0) and always submits both fields', () => {
    const payload = transformFormDataToPayload(formValues())
    expect(payload.rpm).toBe(0)
    expect(payload.concurrency).toBe(0)
  })

  test('submits configured rpm and concurrency', () => {
    const payload = transformFormDataToPayload(
      formValues({ rpm: 60, concurrency: 2 })
    )
    expect(payload.rpm).toBe(60)
    expect(payload.concurrency).toBe(2)
  })

  test('rejects negative values with the backend wording', () => {
    expect(keyIssues({ rpm: -1 })).toContain('Rate limits must not be negative')
    expect(keyIssues({ concurrency: -5 })).toContain(
      'Rate limits must not be negative'
    )
    expect(keyIssues({ rpm: 0, concurrency: 0 })).toHaveLength(0)
  })

  test('backfills stored limits when editing', () => {
    const defaults = transformApiKeyToFormDefaults({
      id: 1,
      name: 'cli',
      key: VALID_KEY,
      status: 1,
      remain_quota: 0,
      used_quota: 0,
      request_count: 0,
      used_tokens: 0,
      cache_read_tokens: 0,
      unlimited_quota: true,
      expired_time: -1,
      created_time: 1,
      accessed_time: 0,
      group: 'default',
      auto_groups: null,
      cross_group_retry: false,
      model_limits_enabled: false,
      model_limits: '',
      allow_ips: '',
      allow_user_quota_query: true,
      rpm: 30,
      concurrency: 1,
    })
    expect(defaults.rpm).toBe(30)
    expect(defaults.concurrency).toBe(1)
  })

  test('falls back to unlimited when the api omits the fields', () => {
    // 模拟旧版后端响应不携带限流字段
    const legacy = {
      id: 1,
      name: 'cli',
      key: VALID_KEY,
      status: 1,
      remain_quota: 0,
      used_quota: 0,
      request_count: 0,
      used_tokens: 0,
      cache_read_tokens: 0,
      unlimited_quota: true,
      expired_time: -1,
      created_time: 1,
      accessed_time: 0,
      group: 'default',
      auto_groups: null,
      cross_group_retry: false,
      model_limits_enabled: false,
      model_limits: '',
      allow_ips: '',
      allow_user_quota_query: true,
    } as unknown as ApiKey
    const defaults = transformApiKeyToFormDefaults(legacy)
    expect(defaults.rpm).toBe(0)
    expect(defaults.concurrency).toBe(0)
  })
})
