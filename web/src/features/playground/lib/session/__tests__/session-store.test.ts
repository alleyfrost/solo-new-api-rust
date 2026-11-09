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
import { beforeEach, expect, test } from 'vitest'

import { useAuthStore, type AuthUser } from '@/stores/auth-store'

import {
  DEFAULT_CONFIG,
  DEFAULT_PARAMETER_ENABLED,
  STORAGE_KEYS,
} from '../../../constants'
import { STORAGE_VERSION } from '../../storage/storage-schema'
import {
  createSessionMeta,
  deleteSessionData,
  loadSessionIndex,
  loadSessionMessages,
  readDrafts,
  saveDrafts,
  saveSessionIndex,
  sessionTitleFromText,
  clearAllSessionData,
} from '../session-store'

const USER_ID = 7
const ENVELOPE_VERSION = STORAGE_VERSION

function envelope(data: unknown) {
  return JSON.stringify({ version: ENVELOPE_VERSION, data })
}

function signIn(id: number = USER_ID) {
  useAuthStore.getState().auth.setUser({
    id,
    username: 'root',
    role: 100,
  } as AuthUser)
}

function message(key: string, content: string) {
  return { key, from: 'user', versions: [{ id: `${key}-v`, content }] }
}

beforeEach(() => {
  localStorage.clear()
  signIn()
})

test('legacy single conversation migrates into the first session', async () => {
  localStorage.setItem(
    `${STORAGE_KEYS.MESSAGES}:${USER_ID}`,
    envelope([message('old', '旧对话')])
  )
  localStorage.setItem(
    `${STORAGE_KEYS.CONFIG}:${USER_ID}`,
    envelope({ model: 'migrated-model' })
  )

  const index = loadSessionIndex()

  expect(index.sessions).toHaveLength(1)
  expect(index.activeId).toBe(index.sessions[0].id)
  expect(index.sessions[0].config.model).toBe('migrated-model')
  expect(localStorage.getItem(STORAGE_KEYS.MESSAGES)).toBeNull()
  expect(localStorage.getItem(`${STORAGE_KEYS.MESSAGES}:${USER_ID}`)).toBeNull()

  const messages = await loadSessionMessages(index.sessions[0].id)
  expect(messages[0].versions[0].content).toBe('旧对话')
})

test('index round-trips and falls back to the first session for a stale activeId', () => {
  const first = createSessionMeta(
    { ...DEFAULT_CONFIG, model: 'a' },
    DEFAULT_PARAMETER_ENABLED,
    1
  )
  const second = createSessionMeta(DEFAULT_CONFIG, DEFAULT_PARAMETER_ENABLED, 2)
  saveSessionIndex({
    activeId: 'missing',
    sessions: [first, second],
  })

  const loaded = loadSessionIndex()

  expect(loaded.sessions.map((session) => session.id)).toEqual([
    first.id,
    second.id,
  ])
  expect(loaded.activeId).toBe(first.id)
})

test('drafts are stored per session and pruned with the session', async () => {
  saveDrafts({ a: '草稿 A', b: '草稿 B' })
  expect(readDrafts()).toEqual({ a: '草稿 A', b: '草稿 B' })

  await deleteSessionData('a')

  expect(readDrafts()).toEqual({ b: '草稿 B' })
})

test('clearing playground data removes sessions, drafts and messages', async () => {
  const session = createSessionMeta({ model: 'x' } as never, {} as never)
  saveSessionIndex({ activeId: session.id, sessions: [session] })
  saveDrafts({ [session.id]: 'hi' })

  await clearAllSessionData()

  expect(localStorage.length).toBe(0)
  expect(readDrafts()).toEqual({})
  expect(loadSessionIndex().sessions).toHaveLength(1)
})

test('titles come from the first user message and are capped', () => {
  expect(sessionTitleFromText('  画一只猫  在雪山上 ')).toBe(
    '画一只猫 在雪山上'
  )
  expect(sessionTitleFromText('一'.repeat(30))).toHaveLength(20)
})
