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
import { act, renderHook, waitFor } from '@testing-library/react'
import { beforeEach, expect, test } from 'vitest'

import { useAuthStore, type AuthUser } from '@/stores/auth-store'

import {
  DEFAULT_CONFIG,
  DEFAULT_PARAMETER_ENABLED,
  MESSAGE_STATUS,
  STORAGE_KEYS,
} from '../../constants'
import type { Message } from '../../types'
import { usePlaygroundState } from '../use-playground-state'

const USER_ID = 21

function signIn() {
  useAuthStore.getState().auth.setUser({
    id: USER_ID,
    username: 'root',
    role: 100,
  } as AuthUser)
}

function envelope(data: unknown) {
  return JSON.stringify({ version: 1, data })
}

function writeSession(
  id: string,
  title: string,
  messages: Message[],
  model: string,
  updatedAt: number
) {
  localStorage.setItem(
    `playground_session_messages:${USER_ID}:${id}`,
    envelope(messages)
  )
  const meta = {
    id,
    title,
    createdAt: updatedAt,
    updatedAt,
    config: { ...DEFAULT_CONFIG, model },
    parameterEnabled: DEFAULT_PARAMETER_ENABLED,
  }
  return meta
}

function writeIndex(sessions: unknown[], activeId: string | null) {
  localStorage.setItem(
    `playground_session_index:${USER_ID}`,
    envelope({ activeId, sessions })
  )
}

function userMessage(key: string, content: string): Message {
  return { key, from: 'user', versions: [{ id: `${key}-v`, content }] }
}

function pendingAssistant(key: string): Message {
  return {
    key,
    from: 'assistant',
    versions: [{ id: `${key}-v`, content: '' }],
    status: MESSAGE_STATUS.LOADING,
    startedAt: Date.now(),
  }
}

beforeEach(() => {
  localStorage.clear()
  signIn()
})

test('migrates the legacy conversation into the first session', async () => {
  localStorage.setItem(
    `${STORAGE_KEYS.MESSAGES}:${USER_ID}`,
    envelope([userMessage('old', '旧对话')])
  )

  const { result } = renderHook(() => usePlaygroundState())

  await waitFor(() => expect(result.current.isLoadingMessages).toBe(false))
  expect(result.current.sessions).toHaveLength(1)
  expect(result.current.messages[0].versions[0].content).toBe('旧对话')
  expect(localStorage.getItem(`${STORAGE_KEYS.MESSAGES}:${USER_ID}`)).toBeNull()
})

test('switching sessions restores their messages, model and draft', async () => {
  const now = Date.now()
  const first = writeSession(
    'a',
    '会话 A',
    [userMessage('a1', 'A 内容')],
    'model-a',
    now
  )
  const second = writeSession(
    'b',
    '会话 B',
    [userMessage('b1', 'B 内容')],
    'model-b',
    now - 1000
  )
  writeIndex([first, second], 'a')
  localStorage.setItem(
    `playground_session_drafts:${USER_ID}`,
    envelope({ a: '草稿 A', b: '草稿 B' })
  )

  const { result } = renderHook(() => usePlaygroundState())
  await waitFor(() => expect(result.current.isLoadingMessages).toBe(false))
  expect(result.current.config.model).toBe('model-a')
  expect(result.current.draft).toBe('草稿 A')

  act(() => result.current.selectSession('b'))

  await waitFor(() => expect(result.current.activeId).toBe('b'))
  await waitFor(() => expect(result.current.config.model).toBe('model-b'))
  await waitFor(() =>
    expect(result.current.messages[0]?.versions[0].content).toBe('B 内容')
  )
  expect(result.current.draft).toBe('草稿 B')
})

test('the first user message becomes the session title and persists', async () => {
  const meta = writeSession('a', '', [], 'model-a', Date.now())
  writeIndex([meta], 'a')

  const { result } = renderHook(() => usePlaygroundState())
  await waitFor(() => expect(result.current.isLoadingMessages).toBe(false))

  act(() => {
    result.current.updateMessages([userMessage('u1', '画一只猫')])
  })

  await waitFor(
    () => expect(result.current.sessions[0].title).toBe('画一只猫'),
    { timeout: 3000 }
  )
  const stored = JSON.parse(
    localStorage.getItem(`playground_session_messages:${USER_ID}:a`) ?? '{}'
  )
  expect(stored.data).toHaveLength(1)
})

test('streamed updates land on the originating conversation while another is active', async () => {
  const now = Date.now()
  const first = writeSession(
    'a',
    '会话 A',
    [userMessage('a1', 'A 内容')],
    'model-a',
    now
  )
  const second = writeSession('b', '会话 B', [], 'model-b', now - 1000)
  writeIndex([first, second], 'a')

  const { result } = renderHook(() => usePlaygroundState())
  await waitFor(() => expect(result.current.isLoadingMessages).toBe(false))
  expect(result.current.messages).toHaveLength(1)

  act(() => {
    result.current.updateMessages([
      ...result.current.messages,
      pendingAssistant('a2'),
    ])
  })

  act(() => result.current.selectSession('b'))
  await waitFor(() => expect(result.current.activeId).toBe('b'))
  await waitFor(() => expect(result.current.messages).toEqual([]))

  act(() => {
    result.current.updateSessionMessages('a', (prev) =>
      prev.map((message) =>
        message.key === 'a2'
          ? {
              ...message,
              versions: [{ id: 'a2-v', content: 'A 流式内容' }],
              status: MESSAGE_STATUS.COMPLETE,
            }
          : message
      )
    )
  })

  // The active conversation is untouched by the background write.
  expect(result.current.messages).toEqual([])

  await waitFor(
    () => {
      const stored = JSON.parse(
        localStorage.getItem(`playground_session_messages:${USER_ID}:a`) ?? '{}'
      )
      expect(stored.data?.[1]?.versions?.[0]?.content).toBe('A 流式内容')
    },
    { timeout: 3000 }
  )

  act(() => result.current.selectSession('a'))
  await waitFor(() => expect(result.current.activeId).toBe('a'))
  await waitFor(() =>
    expect(result.current.messages[1]?.versions[0]?.content).toBe('A 流式内容')
  )
})

test('a new conversation inherits the current config and becomes active', async () => {
  const meta = writeSession('a', '会话 A', [], 'model-a', Date.now())
  writeIndex([meta], 'a')

  const { result } = renderHook(() => usePlaygroundState())
  await waitFor(() => expect(result.current.isLoadingMessages).toBe(false))

  act(() => {
    result.current.updateConfig('model', 'model-new')
  })
  act(() => {
    result.current.createSession()
  })

  await waitFor(() => expect(result.current.sessions).toHaveLength(2))
  expect(result.current.activeId).toBe(result.current.sessions[0].id)
  expect(result.current.config.model).toBe('model-new')
  expect(result.current.messages).toEqual([])
})
