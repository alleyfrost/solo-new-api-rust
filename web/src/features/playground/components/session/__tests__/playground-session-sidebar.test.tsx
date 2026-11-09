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
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import i18next from 'i18next'
import type { ComponentProps } from 'react'
import { beforeAll, expect, test, vi } from 'vitest'

import { DEFAULT_CONFIG, DEFAULT_PARAMETER_ENABLED } from '../../../constants'
import { createSessionMeta } from '../../../lib/session/session-store'
import { PlaygroundSessionSidebar } from '../playground-session-sidebar'

const LABELS = [
  'Actions',
  'Cancel',
  'Collapse sidebar',
  'Conversations',
  'Delete',
  'Delete conversation',
  'Earlier',
  'Expand sidebar',
  'Generating...',
  'New chat',
  'New conversation',
  'Rename',
  'Search conversations',
  'This conversation will be permanently deleted.',
  'Today',
  'Yesterday',
]

beforeAll(() => {
  i18next.addResourceBundle(
    'en',
    'translation',
    Object.fromEntries(LABELS.map((label) => [label, label]))
  )
})

function session(title: string, updatedAt: number, id = title) {
  const meta = createSessionMeta(
    DEFAULT_CONFIG,
    DEFAULT_PARAMETER_ENABLED,
    updatedAt
  )
  return { ...meta, id, title, updatedAt }
}

function renderSidebar(
  overrides: Partial<ComponentProps<typeof PlaygroundSessionSidebar>> = {}
) {
  const props = {
    sessions: [],
    activeId: null,
    collapsed: false,
    onToggle: vi.fn(),
    onCreate: vi.fn(),
    onSelect: vi.fn(),
    onRename: vi.fn(),
    onDelete: vi.fn(),
    ...overrides,
  } satisfies ComponentProps<typeof PlaygroundSessionSidebar>
  return { ...render(<PlaygroundSessionSidebar {...props} />), props }
}

test('groups conversations by day and highlights the active one', () => {
  const now = Date.now()
  renderSidebar({
    sessions: [
      session('今天的会话', now, 'today'),
      session('昨天的会话', now - 86400000, 'yesterday'),
      session('更早的会话', now - 3 * 86400000, 'earlier'),
    ],
    activeId: 'today',
  })

  expect(screen.getByText('Today')).toBeInTheDocument()
  expect(screen.getByText('Yesterday')).toBeInTheDocument()
  expect(screen.getByText('Earlier')).toBeInTheDocument()
  expect(
    screen.getByText('今天的会话').closest('[role="button"]')
  ).toHaveAttribute('aria-current', 'true')
})

test('selects a conversation', async () => {
  const user = userEvent.setup()
  const { props } = renderSidebar({
    sessions: [session('A', Date.now(), 'a')],
  })

  await user.click(screen.getByText('A'))
  expect(props.onSelect).toHaveBeenCalledWith('a')
})

test('marks a generating conversation and still allows switching away', async () => {
  const user = userEvent.setup()
  const { props } = renderSidebar({
    sessions: [
      session('运行中', Date.now(), 'a'),
      session('空闲', Date.now(), 'b'),
    ],
    activeId: 'a',
    runningSessionIds: new Set(['a']),
  })

  expect(screen.getByLabelText('Generating...')).toBeInTheDocument()

  await user.click(screen.getByText('空闲'))
  expect(props.onSelect).toHaveBeenCalledWith('b')
})

test('renames a conversation inline', async () => {
  const user = userEvent.setup()
  const { props } = renderSidebar({
    sessions: [session('旧标题', Date.now(), 'a')],
  })

  await user.dblClick(screen.getByText('旧标题'))
  const input = screen.getByDisplayValue('旧标题')
  await user.clear(input)
  await user.type(input, '新标题{Enter}')

  expect(props.onRename).toHaveBeenCalledWith('a', '新标题')
})

test('deletes a conversation after confirmation', async () => {
  const user = userEvent.setup()
  const { props } = renderSidebar({
    sessions: [session('待删除', Date.now(), 'a')],
  })

  await user.click(screen.getByLabelText('Actions'))
  await user.click(await screen.findByText('Delete'))
  await user.click(await screen.findByRole('button', { name: 'Delete' }))

  await waitFor(() => expect(props.onDelete).toHaveBeenCalledWith('a'))
})
