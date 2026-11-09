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
import { useCallback, useEffect, useRef, useState } from 'react'

import { DEFAULT_CONFIG, DEFAULT_PARAMETER_ENABLED } from '../constants'
import { removeInlineImages } from '../lib/message/message-images'
import { getMessageContent } from '../lib/message/message-utils'
import {
  createSessionMeta,
  deleteSessionData,
  loadSessionIndex,
  loadSessionMessages,
  MAX_SESSIONS,
  persistSessionMessages,
  readDrafts,
  saveDrafts,
  saveSessionIndex,
  sessionTitleFromText,
  type SessionDrafts,
  type SessionIndex,
} from '../lib/session/session-store'
import {
  applyMessageStateUpdate,
  type MessageStateUpdater,
} from '../lib/state/playground-state-utils'
import type {
  GroupOption,
  Message,
  ModelOption,
  ParameterEnabled,
  PlaygroundConfig,
} from '../types'

const MESSAGE_SAVE_DEBOUNCE_MS = 500
const DRAFT_SAVE_DEBOUNCE_MS = 300

function withActiveSession(index: SessionIndex): SessionIndex {
  if (index.sessions.length === 0) {
    const session = createSessionMeta(DEFAULT_CONFIG, DEFAULT_PARAMETER_ENABLED)
    return { activeId: session.id, sessions: [session] }
  }
  if (!index.sessions.some((session) => session.id === index.activeId)) {
    return { ...index, activeId: index.sessions[0].id }
  }
  return index
}

/**
 * Session state for the playground: each conversation owns its messages,
 * model config and unsent draft; only the active one is loaded into memory.
 */
export function usePlaygroundState() {
  const [index, setIndex] = useState<SessionIndex>(() =>
    withActiveSession(loadSessionIndex())
  )
  const indexRef = useRef(index)
  indexRef.current = index

  const initialMeta = index.sessions.find(
    (session) => session.id === index.activeId
  )
  const [config, setConfig] = useState<PlaygroundConfig>(
    initialMeta?.config ?? DEFAULT_CONFIG
  )
  const configRef = useRef(config)
  configRef.current = config

  const [parameterEnabled, setParameterEnabled] = useState<ParameterEnabled>(
    initialMeta?.parameterEnabled ?? DEFAULT_PARAMETER_ENABLED
  )
  const parameterEnabledRef = useRef(parameterEnabled)
  parameterEnabledRef.current = parameterEnabled

  const [messages, setMessages] = useState<Message[]>([])
  const messagesRef = useRef<Message[]>([])
  const [draft, setDraft] = useState('')
  const draftRef = useRef('')
  const draftsRef = useRef<SessionDrafts>(readDrafts())
  const [isLoadingMessages, setIsLoadingMessages] = useState(true)
  const loadedRef = useRef(false)

  const [models, setModels] = useState<ModelOption[]>([])
  const [groups, setGroups] = useState<GroupOption[]>([])

  const messageTimerRef = useRef<number | null>(null)
  const messagePendingRef = useRef<{
    sessionId: string
    messages: Message[]
  } | null>(null)
  const writeChainRef = useRef<Promise<void>>(Promise.resolve())
  const draftTimerRef = useRef<number | null>(null)
  const loadTokenRef = useRef(0)

  const commitIndex = useCallback((next: SessionIndex) => {
    indexRef.current = next
    setIndex(next)
    saveSessionIndex(next)
  }, [])

  const persistDrafts = useCallback(() => {
    if (draftTimerRef.current !== null) {
      window.clearTimeout(draftTimerRef.current)
      draftTimerRef.current = null
    }
    const sessionId = indexRef.current.activeId
    if (!sessionId) return
    const next = { ...draftsRef.current }
    if (draftRef.current) {
      next[sessionId] = draftRef.current
    } else {
      delete next[sessionId]
    }
    draftsRef.current = next
    saveDrafts(next)
  }, [])

  const updateDraft = useCallback(
    (text: string) => {
      draftRef.current = text
      setDraft(text)
      if (draftTimerRef.current !== null) {
        window.clearTimeout(draftTimerRef.current)
      }
      draftTimerRef.current = window.setTimeout(() => {
        draftTimerRef.current = null
        persistDrafts()
      }, DRAFT_SAVE_DEBOUNCE_MS)
    },
    [persistDrafts]
  )

  const touchSession = useCallback(
    (sessionId: string, list: Message[]) => {
      const current = indexRef.current
      const meta = current.sessions.find((session) => session.id === sessionId)
      if (!meta) return
      const firstUser = list.find((message) => message.from === 'user')
      const derived = firstUser
        ? sessionTitleFromText(removeInlineImages(getMessageContent(firstUser)))
        : ''
      const title = meta.title || derived
      const now = Date.now()
      if (meta.title === title && now - meta.updatedAt < 1000) {
        return
      }
      commitIndex({
        ...current,
        sessions: current.sessions.map((session) =>
          session.id === sessionId
            ? { ...session, title, updatedAt: now }
            : session
        ),
      })
    },
    [commitIndex]
  )

  const enqueuePersist = useCallback((sessionId: string, list: Message[]) => {
    writeChainRef.current = writeChainRef.current
      .then(() => persistSessionMessages(sessionId, list))
      .catch(() => undefined)
    return writeChainRef.current
  }, [])

  const flushMessages = useCallback(async () => {
    if (messageTimerRef.current !== null) {
      window.clearTimeout(messageTimerRef.current)
      messageTimerRef.current = null
    }
    const pending = messagePendingRef.current
    messagePendingRef.current = null
    if (!pending) {
      await writeChainRef.current
      return
    }
    touchSession(pending.sessionId, pending.messages)
    await enqueuePersist(pending.sessionId, pending.messages)
  }, [enqueuePersist, touchSession])

  const scheduleMessageSave = useCallback(
    (sessionId: string, list: Message[]) => {
      if (!loadedRef.current) return
      messagePendingRef.current = { sessionId, messages: list }
      if (messageTimerRef.current !== null) {
        window.clearTimeout(messageTimerRef.current)
      }
      messageTimerRef.current = window.setTimeout(() => {
        messageTimerRef.current = null
        void flushMessages()
      }, MESSAGE_SAVE_DEBOUNCE_MS)
    },
    [flushMessages]
  )

  const updateMessages = useCallback(
    (updater: MessageStateUpdater) => {
      const next = applyMessageStateUpdate(messagesRef.current, updater)
      messagesRef.current = next
      setMessages(next)
      const sessionId = indexRef.current.activeId
      if (sessionId) {
        scheduleMessageSave(sessionId, next)
      }
    },
    [scheduleMessageSave]
  )

  const patchActiveMeta = useCallback(
    (patch: {
      config?: PlaygroundConfig
      parameterEnabled?: ParameterEnabled
    }) => {
      const current = indexRef.current
      const sessionId = current.activeId
      if (!sessionId) return
      commitIndex({
        ...current,
        sessions: current.sessions.map((session) =>
          session.id === sessionId ? { ...session, ...patch } : session
        ),
      })
    },
    [commitIndex]
  )

  const updateConfig = useCallback(
    <K extends keyof PlaygroundConfig>(key: K, value: PlaygroundConfig[K]) => {
      const next = { ...configRef.current, [key]: value }
      configRef.current = next
      setConfig(next)
      patchActiveMeta({ config: next })
    },
    [patchActiveMeta]
  )

  const updateParameterEnabled = useCallback(
    (key: keyof ParameterEnabled, value: boolean) => {
      const next = { ...parameterEnabledRef.current, [key]: value }
      parameterEnabledRef.current = next
      setParameterEnabled(next)
      patchActiveMeta({ parameterEnabled: next })
    },
    [patchActiveMeta]
  )

  // Persist the freshly created or migrated index so the active session sticks.
  useEffect(() => {
    saveSessionIndex(indexRef.current)
  }, [])

  const activeId = index.activeId
  useEffect(() => {
    if (!activeId) {
      messagesRef.current = []
      setMessages([])
      setIsLoadingMessages(false)
      return
    }
    const token = loadTokenRef.current + 1
    loadTokenRef.current = token
    loadedRef.current = false
    setMessages([])
    messagesRef.current = []
    setIsLoadingMessages(true)
    void (async () => {
      // Never read a session before its pending debounced write has landed.
      await flushMessages()
      if (token !== loadTokenRef.current) return
      const loaded = await loadSessionMessages(activeId)
      if (token !== loadTokenRef.current) return
      messagesRef.current = loaded
      setMessages(loaded)
      const storedDraft = draftsRef.current[activeId] ?? ''
      draftRef.current = storedDraft
      setDraft(storedDraft)
      loadedRef.current = true
      setIsLoadingMessages(false)
    })()
  }, [activeId, flushMessages])

  useEffect(
    () => () => {
      void flushMessages()
      persistDrafts()
    },
    [flushMessages, persistDrafts]
  )

  const selectSession = useCallback(
    (sessionId: string) => {
      const current = indexRef.current
      if (sessionId === current.activeId) return
      const meta = current.sessions.find((session) => session.id === sessionId)
      if (!meta) return
      void flushMessages()
      persistDrafts()
      commitIndex({ ...current, activeId: sessionId })
      configRef.current = meta.config
      setConfig(meta.config)
      parameterEnabledRef.current = meta.parameterEnabled
      setParameterEnabled(meta.parameterEnabled)
    },
    [commitIndex, flushMessages, persistDrafts]
  )

  const createSession = useCallback((): boolean => {
    const current = indexRef.current
    const active = current.sessions.find(
      (session) => session.id === current.activeId
    )
    if (active && messagesRef.current.length === 0 && !active.title) {
      return true
    }
    if (current.sessions.length >= MAX_SESSIONS) {
      return false
    }
    void flushMessages()
    persistDrafts()
    const meta = createSessionMeta(
      configRef.current,
      parameterEnabledRef.current
    )
    commitIndex({ activeId: meta.id, sessions: [meta, ...current.sessions] })
    return true
  }, [commitIndex, flushMessages, persistDrafts])

  const renameSession = useCallback(
    (sessionId: string, title: string) => {
      const current = indexRef.current
      commitIndex({
        ...current,
        sessions: current.sessions.map((session) =>
          session.id === sessionId
            ? { ...session, title: title.trim() }
            : session
        ),
      })
    },
    [commitIndex]
  )

  const deleteSession = useCallback(
    async (sessionId: string) => {
      await flushMessages()
      persistDrafts()
      const current = indexRef.current
      let sessions = current.sessions.filter(
        (session) => session.id !== sessionId
      )
      let nextActiveId = current.activeId
      if (nextActiveId === sessionId) {
        const sorted = [...sessions].sort(
          (left, right) => right.updatedAt - left.updatedAt
        )
        nextActiveId = sorted[0]?.id ?? null
      }
      if (sessions.length === 0) {
        const meta = createSessionMeta(
          configRef.current,
          parameterEnabledRef.current
        )
        sessions = [meta]
        nextActiveId = meta.id
      }
      commitIndex({ activeId: nextActiveId, sessions })
      const nextMeta = sessions.find((session) => session.id === nextActiveId)
      if (nextMeta) {
        configRef.current = nextMeta.config
        setConfig(nextMeta.config)
        parameterEnabledRef.current = nextMeta.parameterEnabled
        setParameterEnabled(nextMeta.parameterEnabled)
      }
      const drafts = { ...draftsRef.current }
      delete drafts[sessionId]
      draftsRef.current = drafts
      saveDrafts(drafts)
      await deleteSessionData(sessionId)
    },
    [commitIndex, flushMessages, persistDrafts]
  )

  const clearMessages = useCallback(() => {
    updateMessages([])
  }, [updateMessages])

  const resetConfig = useCallback(() => {
    configRef.current = DEFAULT_CONFIG
    setConfig(DEFAULT_CONFIG)
    parameterEnabledRef.current = DEFAULT_PARAMETER_ENABLED
    setParameterEnabled(DEFAULT_PARAMETER_ENABLED)
    patchActiveMeta({
      config: DEFAULT_CONFIG,
      parameterEnabled: DEFAULT_PARAMETER_ENABLED,
    })
  }, [patchActiveMeta])

  return {
    config,
    parameterEnabled,
    messages,
    isLoadingMessages,
    models,
    groups,
    sessions: index.sessions,
    activeId: index.activeId,
    draft,
    canCreateSession: index.sessions.length < MAX_SESSIONS,

    setModels,
    setGroups,

    updateConfig,
    updateParameterEnabled,
    updateMessages,
    updateDraft,
    selectSession,
    createSession,
    renameSession,
    deleteSession,
    clearMessages,
    resetConfig,
  }
}
