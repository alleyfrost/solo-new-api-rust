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
import { nanoid } from 'nanoid'
import { z } from 'zod'

import {
  DEFAULT_CONFIG,
  DEFAULT_PARAMETER_ENABLED,
  STORAGE_KEYS,
} from '../../constants'
import type { Message, ParameterEnabled, PlaygroundConfig } from '../../types'
import {
  cacheSessionImages,
  deleteSessionImages,
  deleteUserImages,
  rehydrateSessionImages,
} from '../storage/image-cache'
import {
  getPlaygroundUserId,
  loadConfig,
  loadMessages,
  loadParameterEnabled,
  normalizeMessages,
  toStoredMessages,
} from '../storage/storage'
import {
  parameterEnabledSchema,
  playgroundConfigSchema,
  STORAGE_VERSION,
} from '../storage/storage-schema'

export const MAX_SESSIONS = 50

export type SessionMeta = {
  id: string
  title: string
  createdAt: number
  updatedAt: number
  config: PlaygroundConfig
  parameterEnabled: ParameterEnabled
}

export type SessionIndex = {
  activeId: string | null
  sessions: SessionMeta[]
}

export type SessionDrafts = Record<string, string>

const sessionMetaSchema = z.object({
  id: z.string().min(1),
  title: z.string(),
  createdAt: z.number(),
  updatedAt: z.number(),
  config: playgroundConfigSchema,
  parameterEnabled: parameterEnabledSchema,
})

const sessionIndexSchema = z.object({
  activeId: z.string().nullable(),
  sessions: z.array(sessionMetaSchema),
})

type Envelope<T> = { version: number; data: T }

function storage(): Storage | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage
  } catch {
    return null
  }
}

function indexKey(userId: number): string {
  return `playground_session_index:${userId}`
}

function messagesKey(userId: number, sessionId: string): string {
  return `playground_session_messages:${userId}:${sessionId}`
}

function draftsKey(userId: number): string {
  return `playground_session_drafts:${userId}`
}

function readJson<T>(key: string): T | null {
  const store = storage()
  if (!store) return null
  try {
    const saved = store.getItem(key)
    return saved ? (JSON.parse(saved) as T) : null
  } catch {
    return null
  }
}

function unwrap(value: unknown): unknown {
  if (
    value &&
    typeof value === 'object' &&
    'version' in value &&
    'data' in value
  ) {
    return (value as Envelope<unknown>).data
  }
  return value
}

function writeJson<T>(key: string, data: T): void {
  const store = storage()
  if (!store) return
  try {
    store.setItem(key, JSON.stringify({ version: STORAGE_VERSION, data }))
  } catch {
    // Quota exceeded: keep the in-memory state, the next save may succeed.
  }
}

function normalizeMeta(meta: z.infer<typeof sessionMetaSchema>): SessionMeta {
  return {
    id: meta.id,
    title: meta.title ?? '',
    createdAt: meta.createdAt,
    updatedAt: meta.updatedAt,
    config: { ...DEFAULT_CONFIG, ...meta.config },
    parameterEnabled: {
      ...DEFAULT_PARAMETER_ENABLED,
      ...meta.parameterEnabled,
    },
  }
}

function sanitizeIndex(raw: unknown): SessionIndex | null {
  const parsed = sessionIndexSchema.safeParse(unwrap(raw))
  if (!parsed.success) {
    return null
  }
  const seen = new Set<string>()
  const sessions = parsed.data.sessions.map(normalizeMeta).filter((session) => {
    if (seen.has(session.id)) return false
    seen.add(session.id)
    return true
  })
  const activeId = sessions.some(
    (session) => session.id === parsed.data.activeId
  )
    ? parsed.data.activeId
    : (sessions[0]?.id ?? null)
  return { activeId, sessions }
}

function clearLegacyKeys(userId: number): void {
  const store = storage()
  if (!store) return
  for (const baseKey of Object.values(STORAGE_KEYS)) {
    store.removeItem(baseKey)
    store.removeItem(`${baseKey}:${userId}`)
  }
}

function migrateLegacyData(userId: number): SessionIndex {
  const legacyMessages = loadMessages()
  const config = { ...DEFAULT_CONFIG, ...loadConfig() }
  const parameterEnabled = {
    ...DEFAULT_PARAMETER_ENABLED,
    ...loadParameterEnabled(),
  }
  const now = Date.now()
  const session: SessionMeta = {
    id: nanoid(),
    title: '',
    createdAt: now,
    updatedAt: now,
    config,
    parameterEnabled,
  }
  if (legacyMessages && legacyMessages.length > 0) {
    writeJson(messagesKey(userId, session.id), toStoredMessages(legacyMessages))
  }
  const index: SessionIndex = { activeId: session.id, sessions: [session] }
  writeJson(indexKey(userId), index)
  clearLegacyKeys(userId)
  return index
}

export function loadSessionIndex(): SessionIndex {
  const userId = getPlaygroundUserId()
  if (userId == null) {
    return { activeId: null, sessions: [] }
  }
  const saved = readJson<unknown>(indexKey(userId))
  const sanitized = saved ? sanitizeIndex(saved) : null
  return sanitized ?? migrateLegacyData(userId)
}

export function saveSessionIndex(index: SessionIndex): void {
  const userId = getPlaygroundUserId()
  if (userId == null) return
  writeJson(indexKey(userId), index)
}

export function createSessionMeta(
  config: PlaygroundConfig,
  parameterEnabled: ParameterEnabled,
  now: number = Date.now()
): SessionMeta {
  return {
    id: nanoid(),
    title: '',
    createdAt: now,
    updatedAt: now,
    config: { ...config },
    parameterEnabled: { ...parameterEnabled },
  }
}

export function sessionTitleFromText(text: string): string {
  return text.replaceAll(/\s+/g, ' ').trim().slice(0, 20)
}

function readStoredMessages(userId: number, sessionId: string): Message[] {
  const saved = readJson<unknown>(messagesKey(userId, sessionId))
  if (!saved) return []
  try {
    return normalizeMessages(unwrap(saved))
  } catch {
    return []
  }
}

export async function loadSessionMessages(
  sessionId: string
): Promise<Message[]> {
  const userId = getPlaygroundUserId()
  if (userId == null) return []
  const messages = readStoredMessages(userId, sessionId)
  if (messages.length === 0) return messages
  try {
    return await rehydrateSessionImages(userId, sessionId, messages)
  } catch {
    // IndexedDB unavailable: keep markers, chat stays usable.
    return messages
  }
}

export async function persistSessionMessages(
  sessionId: string,
  messages: Message[]
): Promise<void> {
  const userId = getPlaygroundUserId()
  if (userId == null) return
  try {
    await cacheSessionImages(userId, sessionId, messages)
  } catch {
    // Degrade to page-only images.
  }
  writeJson(messagesKey(userId, sessionId), toStoredMessages(messages))
}

export async function deleteSessionData(sessionId: string): Promise<void> {
  const userId = getPlaygroundUserId()
  if (userId == null) return
  storage()?.removeItem(messagesKey(userId, sessionId))
  const drafts = readDrafts()
  if (sessionId in drafts) {
    delete drafts[sessionId]
    saveDrafts(drafts)
  }
  try {
    await deleteSessionImages(userId, sessionId)
  } catch {
    // Nothing to clean up when IndexedDB is unavailable.
  }
}

export function readDrafts(): SessionDrafts {
  const userId = getPlaygroundUserId()
  if (userId == null) return {}
  const saved = readJson<SessionDrafts>(draftsKey(userId))
  const drafts = unwrap(saved)
  if (!drafts || typeof drafts !== 'object') return {}
  const result: SessionDrafts = {}
  for (const [id, text] of Object.entries(drafts as Record<string, unknown>)) {
    if (typeof text === 'string') {
      result[id] = text
    }
  }
  return result
}

export function saveDrafts(drafts: SessionDrafts): void {
  const userId = getPlaygroundUserId()
  if (userId == null) return
  writeJson(draftsKey(userId), drafts)
}

export async function clearAllSessionData(): Promise<void> {
  const userId = getPlaygroundUserId()
  if (userId == null) return
  const store = storage()
  if (store) {
    store.removeItem(indexKey(userId))
    store.removeItem(draftsKey(userId))
    const prefix = `playground_session_messages:${userId}:`
    const keys: string[] = []
    for (let position = 0; position < store.length; position++) {
      const key = store.key(position)
      if (key?.startsWith(prefix)) {
        keys.push(key)
      }
    }
    for (const key of keys) {
      store.removeItem(key)
    }
  }
  try {
    await deleteUserImages(userId)
  } catch {
    // Nothing to clean up when IndexedDB is unavailable.
  }
  clearLegacyKeys(userId)
}
