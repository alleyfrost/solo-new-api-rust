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
import { render, screen, within } from '@testing-library/react'
import i18next from 'i18next'
import { beforeAll, expect, test } from 'vitest'

import type { UsageLog } from '../../data/schema'
import type { LogOtherData } from '../../types'
import { DetailsDialog } from '../dialogs/details-dialog'

beforeAll(() => {
  // The placeholder-bearing labels must interpolate even when the test
  // instance carries no full locale bundle.
  i18next.addResourceBundle('en', 'translation', {
    'Response model: {{model}}': 'Response model: {{model}}',
  })
})

function makeLog(other: LogOtherData): UsageLog {
  return {
    id: 1,
    user_id: 1,
    created_at: 1,
    type: 2,
    content: '',
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

function renderDetails(
  other: LogOtherData,
  promptTokens = 0,
  completionTokens = 0
) {
  render(
    <DetailsDialog
      log={{
        ...makeLog(other),
        prompt_tokens: promptTokens,
        completion_tokens: completionTokens,
      }}
      isAdmin={false}
      isRoot={false}
      open
      onOpenChange={() => undefined}
    />
  )
}

function rowValue(label: string): string | null {
  return screen.getByText(label).nextElementSibling?.textContent ?? null
}

test('shows the recorded request and response models in log details', () => {
  renderDetails({
    response_model: {
      requested_model: 'requested-model',
      upstream_model: 'mapped-model',
      returned_model: 'unexpected-model',
    },
  })
  expect(screen.getByText('Response model: unexpected-model')).toBeVisible()
  expect(rowValue('Request Model')).toBe('requested-model')
  expect(rowValue('Upstream Model')).toBe('mapped-model')
  const responseModelRow = screen
    .getAllByText('Response Model')
    .find((element) => element.tagName === 'SPAN')
  expect(responseModelRow?.nextElementSibling?.textContent).toContain(
    'unexpected-model'
  )
})

test('splits OpenAI cache writes into 30-minute and 12-hour token counts', () => {
  renderDetails(
    {
      cache_tokens: 2000,
      cache_creation_tokens: 1500,
      cache_creation_tokens_12h: 1000,
    },
    4000
  )
  expect(rowValue('Cache Read')).toBe('2,000')
  expect(rowValue('Cache Write (30m)')).toBe('500')
  expect(rowValue('Cache Write (12h)')).toBe('1,000')
  expect(screen.queryByText('Cache Write')).toBeNull()
  expect(screen.queryByText(/\$/)).toBeNull()
})

test('keeps one cache write row for logs without 12-hour writes', () => {
  renderDetails(
    {
      cache_tokens: 2000,
      cache_creation_tokens: 1500,
    },
    4000
  )
  expect(rowValue('Cache Write')).toBe('1,500')
  expect(screen.queryByText('Cache Write (30m)')).toBeNull()
  expect(screen.queryByText('Cache Write (12h)')).toBeNull()
  expect(screen.queryByText(/\$/)).toBeNull()
})

test('shows input, output and total token counts', () => {
  renderDetails({}, 4000, 100)
  expect(rowValue('Input Tokens')).toBe('4,000')
  expect(rowValue('Output Tokens')).toBe('100')
  expect(rowValue('Total Tokens')).toBe('4,100')
})

test('shows the recorded billable token breakdown under its raw keys', () => {
  renderDetails(
    {
      cache_tokens: 300,
      image_cache_tokens: 200,
      billing_tokens: { p: 300, cr: 100, img: 400, img_cr: 200, c: 100 },
    },
    1000
  )
  expect(rowValue('Cache Read')).toBe('300')
  expect(rowValue('Image Cache')).toBe('200')
  const billable = within(
    screen.getByRole('group', { name: 'Billable token breakdown' })
  )
  expect(billable.getByText('cr').nextElementSibling).toHaveTextContent('100')
  expect(billable.getByText('img').nextElementSibling).toHaveTextContent('400')
  expect(billable.getByText('img_cr').nextElementSibling).toHaveTextContent(
    '200'
  )
  expect(screen.queryByText(/\$/)).toBeNull()
})

test('renders one raw-key row per usage fact in its own section', () => {
  renderDetails({
    usage_facts: {
      resolution: '720P',
      seconds: 5,
    },
  })

  expect(screen.getByText('Usage parameters')).toBeInTheDocument()
  expect(rowValue('resolution')).toBe('720P')
  expect(rowValue('seconds')).toBe('5')
  expect(screen.queryByText('Billing Mode')).toBeNull()
  expect(screen.queryByText('Matched Tier')).toBeNull()
  expect(screen.queryByText('Total Cost')).toBeNull()
})

test('does not render usage parameter rows when usage_facts is absent', () => {
  renderDetails({})

  expect(screen.queryByText('Usage parameters')).toBeNull()
  expect(screen.queryByText('resolution')).toBeNull()
})

test('does not render usage parameter rows when usage_facts is empty', () => {
  renderDetails({ usage_facts: {} })

  expect(screen.queryByText('Usage parameters')).toBeNull()
  expect(screen.queryByText('seconds')).toBeNull()
})
