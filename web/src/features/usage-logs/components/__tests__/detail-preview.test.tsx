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
import {
  flexRender,
  getCoreRowModel,
  useReactTable,
} from '@tanstack/react-table'
import { fireEvent, render, screen, within } from '@testing-library/react'
import { createInstance } from 'i18next'
import type { ReactNode } from 'react'
import { I18nextProvider } from 'react-i18next'
import { afterAll, beforeAll, expect, test, vi } from 'vitest'

import en from '@/i18n/locales/en.json'

import type { UsageLog } from '../../data/schema'
import type { LogOtherData } from '../../types'
import { useCommonLogsColumns } from '../columns/common-logs-columns'

// Provider icons are unused by the log preview; their browser-only
// dependencies cannot be loaded by Vitest's Node ESM resolver.
vi.mock('@lobehub/icons', () => ({}))

vi.hoisted(() => {
  vi.stubGlobal('localStorage', {
    getItem: () => null,
    setItem: () => undefined,
    removeItem: () => undefined,
  })
})
afterAll(() => vi.unstubAllGlobals())

function makeLog(other: LogOtherData, type = 2, content = ''): UsageLog {
  return {
    id: 1,
    user_id: 1,
    created_at: 1,
    type,
    content,
    username: 'user',
    token_name: 'token',
    model_name: 'gpt-4o-mini',
    quota: 0,
    prompt_tokens: 0,
    completion_tokens: 0,
    use_time: 0,
    is_stream: false,
    channel: 1,
    channel_name: '',
    token_id: 1,
    group: 'default',
    ip: '',
    other: JSON.stringify(other),
    request_id: 'req-1',
    upstream_request_id: '',
  }
}

function DetailPreview(props: {
  other: LogOtherData
  isAdmin: boolean
  type?: number
  content?: string
}) {
  const table = useReactTable({
    data: [makeLog(props.other, props.type, props.content)],
    columns: useCommonLogsColumns(props.isAdmin, false),
    getCoreRowModel: getCoreRowModel(),
  })
  const cell = table
    .getRowModel()
    .rows[0].getAllCells()
    .find((item) => item.column.id === 'content')
  if (!cell) throw new Error('The log must have a content column')
  return flexRender(cell.column.columnDef.cell, cell.getContext())
}

const i18n = createInstance()
beforeAll(async () => {
  await i18n.init({
    lng: 'en',
    fallbackLng: 'en',
    resources: { en },
    interpolation: { escapeValue: false },
  })
})

function renderPreview(
  other: LogOtherData,
  isAdmin = true,
  type?: number,
  content?: string
) {
  render(
    <I18nextProvider i18n={i18n}>
      <DetailPreview
        other={other}
        isAdmin={isAdmin}
        type={type}
        content={content}
      />
    </I18nextProvider>
  )
  return screen.getByRole('button', { name: /./ })
}

test('usage log previews stay neutral without any money markers', () => {
  const preview = renderPreview({ cache_tokens: 100, frt: 320 })
  expect(preview.textContent).toBe('—')
})

test('preview falls back to the recorded raw content', () => {
  const preview = renderPreview({}, true, 2, 'Channel test request')
  expect(preview.textContent).toBe('Channel test request')
})

test('system prompt overrides are flagged in the preview', () => {
  const preview = renderPreview({ is_system_prompt_overwritten: true })
  expect(preview.textContent).toBe('System Prompt Override')
})

test('refund logs preview the async task refund marker', () => {
  const preview = renderPreview(
    { task_id: 'task-1', reason: 'failed' },
    true,
    6
  )
  expect(preview.textContent).toBe('Async task refund')
})

test('top-up content_parts render in the viewer language', async () => {
  i18n.addResourceBundle('zh', 'translation', {
    'Online top-up succeeded, amount added: {{quota}}, payment amount: {{amount}}':
      '在线充值成功，充值额度：{{quota}}，支付金额：{{amount}}',
  })
  await i18n.changeLanguage('zh')
  const preview = renderPreview(
    {
      content_parts: [
        {
          key: 'Online top-up succeeded, amount added: {{quota}}, payment amount: {{amount}}',
          params: { quota: '10.000000', amount: '10.00' },
        },
      ],
    },
    false,
    1
  )
  expect(preview.textContent).toBe(
    '在线充值成功，充值额度：10.000000，支付金额：10.00'
  )
  await i18n.changeLanguage('en')
})

test('keeps log details open when the parent refreshes with unchanged data', async () => {
  const other: LogOtherData = { is_system_prompt_overwritten: true }
  const wrapper = ({ children }: { children: ReactNode }) => (
    <I18nextProvider i18n={i18n}>{children}</I18nextProvider>
  )
  const { rerender } = render(<DetailPreview other={other} isAdmin />, {
    wrapper,
  })
  fireEvent.click(
    screen.getByRole('button', { name: 'System Prompt Override' })
  )
  const dialog = await screen.findByRole('dialog')
  expect(dialog).toBeVisible()
  expect(within(dialog).getByText('Overwritten')).toBeVisible()
  rerender(<DetailPreview other={other} isAdmin />)
  expect(screen.getByRole('dialog')).toBeVisible()
})
