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
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import {
  flexRender,
  getCoreRowModel,
  useReactTable,
} from '@tanstack/react-table'
import { act, render, screen, within } from '@testing-library/react'
import { createInstance } from 'i18next'
import { I18nextProvider } from 'react-i18next'
import {
  afterAll,
  afterEach,
  beforeEach,
  describe,
  expect,
  test,
  vi,
} from 'vitest'

import en from '@/i18n/locales/en.json'
import zh from '@/i18n/locales/zh.json'
import {
  DEFAULT_CURRENCY_CONFIG,
  useSystemConfigStore,
} from '@/stores/system-config-store'

import type { UsageLog } from '../../data/schema'
import { renderAuditContent } from '../../lib/format'
import type { LogOtherData } from '../../types'
import { useCommonLogsColumns } from '../columns/common-logs-columns'
import { DetailsDialog } from '../dialogs/details-dialog'

// Provider icons are unused by audit logs; their browser-only dependencies
// cannot be loaded by Vitest's Node ESM resolver.
vi.mock('@lobehub/icons', () => ({}))

vi.hoisted(() => {
  vi.stubGlobal('localStorage', {
    getItem: () => null,
    setItem: () => undefined,
    removeItem: () => undefined,
  })
})

afterAll(() => vi.unstubAllGlobals())

function AuditLogPreview(props: { log: UsageLog }) {
  const table = useReactTable({
    data: [props.log],
    columns: useCommonLogsColumns(false, false),
    getCoreRowModel: getCoreRowModel(),
  })
  const cell = table
    .getRowModel()
    .rows[0].getAllCells()
    .find((item) => item.column.id === 'content')
  if (!cell) throw new Error('The audit log must have a content column')

  return (
    <>
      {flexRender(cell.column.columnDef.cell, cell.getContext())}
      <DetailsDialog
        log={props.log}
        isAdmin={false}
        isRoot={false}
        open
        onOpenChange={() => undefined}
      />
    </>
  )
}

// Go-era quota operations survive as historical rows. They render with the
// raw quota units the backend stored, never as a currency amount.
const cases = [
  {
    action: 'user.quota_add',
    params: {
      target_user_id: 42,
      target_username: 'quota-owner',
      mode: 'add',
      requested_quota: 500000,
      quota: 500000,
      from: 500000,
      to: 1000000,
    },
    english: 'Increased user quota by 500000',
    chinese: '增加用户额度 500000',
  },
  {
    action: 'user.quota_subtract',
    params: { quota: 500000 },
    english: 'Decreased user quota by 500000',
    chinese: '减少用户额度 500000',
  },
  {
    action: 'user.quota_override',
    params: { from: 500000, to: 0 },
    english: 'Overrode user quota from 500000 to 0',
    chinese: '覆盖用户额度，从 500000 改为 0',
  },
  {
    action: 'channel.update',
    params: { name: 'OpenAI', id: 7 },
    english: 'Updated channel OpenAI (ID: 7)',
    chinese: '更新渠道 OpenAI（ID: 7）',
  },
]

describe('audit log localization', () => {
  const previousConfig = useSystemConfigStore.getState().config
  let queryClient: QueryClient

  beforeEach(() => {
    useSystemConfigStore.getState().setConfig({
      currency: { ...DEFAULT_CURRENCY_CONFIG },
    })
    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    queryClient.setQueryData(['status'], {}, { updatedAt: Date.now() + 60_000 })
  })

  afterEach(() => {
    queryClient.clear()
    useSystemConfigStore.getState().setConfig(previousConfig)
  })

  test.each(cases)(
    '$action switches language in the preview and details',
    async (scenario) => {
      const i18n = createInstance()
      await i18n.init({
        lng: 'en',
        fallbackLng: 'en',
        resources: { en, zh },
        interpolation: { escapeValue: false },
      })
      const log: UsageLog = {
        id: 1,
        user_id: 1,
        created_at: 1,
        type: 1,
        content: 'English export fallback',
        username: 'audit-user',
        token_name: '',
        model_name: '',
        quota: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        use_time: 0,
        is_stream: false,
        channel: 0,
        channel_name: '',
        token_id: 0,
        group: '',
        ip: '',
        request_id: 'audit-request',
        upstream_request_id: '',
        other: JSON.stringify({
          op: { action: scenario.action, params: scenario.params },
        }),
      }
      render(
        <I18nextProvider i18n={i18n}>
          <QueryClientProvider client={queryClient}>
            <AuditLogPreview log={log} />
          </QueryClientProvider>
        </I18nextProvider>
      )

      // The modal makes the table preview inert, but both remain rendered.
      expect(screen.getAllByText(scenario.english)).toHaveLength(2)
      expect(
        within(screen.getByRole('dialog')).getByText(scenario.english)
      ).toBeInTheDocument()
      await act(() => i18n.changeLanguage('zh'))
      expect(screen.getAllByText(scenario.chinese)).toHaveLength(2)
      expect(
        within(screen.getByRole('dialog')).getByText(scenario.chinese)
      ).toBeInTheDocument()
      expect(screen.queryByText('English export fallback')).toBeNull()
    }
  )

  test('renders quota units without currency and falls back on unknown actions', async () => {
    const i18n = createInstance()
    await i18n.init({ lng: 'en', resources: { en } })
    const other: LogOtherData = {
      op: {
        action: 'user.quota_add',
        params: { quota: 500_000 },
      },
    }
    const content = renderAuditContent(other, i18n.t)
    expect(content).toBe('Increased user quota by 500000')
    expect(content).not.toContain('$')
    expect(
      renderAuditContent({ op: { action: 'unknown', params: {} } }, i18n.t)
    ).toBeNull()
    expect(renderAuditContent({}, i18n.t)).toBeNull()
  })
})
