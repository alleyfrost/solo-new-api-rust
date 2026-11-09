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
import { act, renderHook, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { expect, test, vi } from 'vitest'

import {
  DEFAULT_CONFIG,
  DEFAULT_PARAMETER_ENABLED,
  MESSAGE_STATUS,
} from '../../constants'
import {
  applyMessageStateUpdate,
  type MessageStateUpdater,
} from '../../lib/state/playground-state-utils'
import type { Message } from '../../types'
import { useChatHandler } from '../use-chat-handler'

const streams = vi.hoisted(() => [] as FakeStream[])

type StreamCallbacks = {
  onUpdate: (type: 'reasoning' | 'content', chunk: string) => void
  onComplete: () => void
  onError: (error: string, errorCode?: string) => void
}

type FakeStream = {
  send: ReturnType<typeof vi.fn>
  stop: ReturnType<typeof vi.fn>
  dispose: ReturnType<typeof vi.fn>
}

vi.mock('../use-stream-request', () => ({
  createPlaygroundStreamRequest: () => {
    const stream = {
      send: vi.fn(),
      stop: vi.fn(),
      dispose: vi.fn(),
    }
    streams.push(stream)
    return stream
  },
}))

vi.mock('@/lib/status-query', () => ({
  statusQueryOptions: {
    queryKey: ['status'],
    queryFn: async () => ({ image_models: [] }),
  },
}))

function pendingAssistant(key: string): Message {
  return {
    key,
    from: 'assistant',
    versions: [{ id: `${key}-v`, content: '' }],
    status: MESSAGE_STATUS.LOADING,
    startedAt: Date.now(),
  }
}

function contentOf(messages: Message[] | undefined): string {
  return messages?.[0]?.versions[0]?.content ?? ''
}

function streamCallbacks(stream: FakeStream): StreamCallbacks {
  return stream.send.mock.calls[0]?.[1] as StreamCallbacks
}

test('keeps concurrent conversations on independent streams', async () => {
  const lists = new Map<string, Message[]>([
    ['a', [pendingAssistant('a1')]],
    ['b', [pendingAssistant('b1')]],
  ])
  const onMessageUpdate = (sessionId: string, updater: MessageStateUpdater) => {
    lists.set(
      sessionId,
      applyMessageStateUpdate(lists.get(sessionId) ?? [], updater)
    )
  }
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  )

  const { result, rerender } = renderHook(
    ({ activeSessionId }: { activeSessionId: string }) =>
      useChatHandler({
        activeSessionId,
        config: { ...DEFAULT_CONFIG, stream: true },
        parameterEnabled: DEFAULT_PARAMETER_ENABLED,
        onMessageUpdate,
      }),
    { wrapper, initialProps: { activeSessionId: 'a' } }
  )

  act(() => result.current.sendChat([pendingAssistant('a1')]))
  expect(streams).toHaveLength(1)
  const streamA = streams[0]

  rerender({ activeSessionId: 'b' })
  act(() => result.current.sendChat([pendingAssistant('b1')]))
  expect(streams).toHaveLength(2)
  const streamB = streams[1]

  act(() => streamCallbacks(streamA).onUpdate('content', 'A1'))
  await waitFor(() => expect(contentOf(lists.get('a'))).toBe('A1'))
  expect(contentOf(lists.get('b'))).toBe('')

  act(() => streamCallbacks(streamB).onUpdate('content', 'B1'))
  await waitFor(() => expect(contentOf(lists.get('b'))).toBe('B1'))
  expect(contentOf(lists.get('a'))).toBe('A1')

  act(() => streamCallbacks(streamA).onComplete())
  await waitFor(() =>
    expect(result.current.runningSessionIds.has('a')).toBe(false)
  )
  expect(result.current.runningSessionIds.has('b')).toBe(true)

  act(() => result.current.stopGeneration('b'))
  expect(streamB.stop).toHaveBeenCalled()
  expect(result.current.runningSessionIds.has('b')).toBe(false)

  act(() => streamCallbacks(streamA).onUpdate('content', 'A2'))
  await waitFor(() => expect(contentOf(lists.get('a'))).toBe('A1A2'))
})
