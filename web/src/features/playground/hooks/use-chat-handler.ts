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
import { useQuery } from '@tanstack/react-query'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { handleServerError } from '@/lib/handle-server-error'
import { statusQueryOptions } from '@/lib/status-query'

import { sendChatCompletion } from '../api'
import { ERROR_MESSAGES } from '../constants'
import {
  applyStreamingChunk,
  buildChatCompletionPayload,
  updateAssistantMessageWithError,
  updateLastAssistantMessage,
  parseRequestErrorDetails,
  applyChatCompletionResponse,
  completeAssistantMessage,
  hasChatCompletionChoice,
  isAssistantMessageFinal,
  isAssistantMessagePending,
} from '../lib'
import type { MessageStateUpdater } from '../lib/state/playground-state-utils'
import type { Message, PlaygroundConfig, ParameterEnabled } from '../types'
import { createPlaygroundStreamRequest } from './use-stream-request'

interface UseChatHandlerOptions {
  activeSessionId: string | null
  config: PlaygroundConfig
  parameterEnabled: ParameterEnabled
  onMessageUpdate: (sessionId: string, updater: MessageStateUpdater) => void
}

const KNOWN_ERROR_MESSAGES = new Set<string>(Object.values(ERROR_MESSAGES))
const STREAM_UPDATE_FLUSH_MS = 50

type PendingStreamChunks = {
  generation: number
  content: string
  reasoning: string
}

type GenerationTask = {
  generation: number
  stream: ReturnType<typeof createPlaygroundStreamRequest> | null
  abortController: AbortController | null
  pending: PendingStreamChunks
  flushTimer: number | null
}

function createGenerationTask(): GenerationTask {
  return {
    generation: 0,
    stream: null,
    abortController: null,
    pending: { generation: 0, content: '', reasoning: '' },
    flushTimer: null,
  }
}

function mergePendingStreamChunk(
  currentChunk: string,
  nextChunk: string
): string {
  if (!currentChunk || !nextChunk.startsWith(currentChunk)) {
    return currentChunk + nextChunk
  }

  return nextChunk
}

/**
 * Hook for handling chat message sending and receiving
 */
export function useChatHandler({
  activeSessionId,
  config,
  parameterEnabled,
  onMessageUpdate,
}: UseChatHandlerOptions) {
  const { t } = useTranslation()
  const { data: status } = useQuery(statusQueryOptions)
  const imageModels = Array.isArray(status?.image_models)
    ? (status.image_models as string[])
    : []
  const imageModel = imageModels.includes(config.model)

  const tasksRef = useRef(new Map<string, GenerationTask>())
  const runningRef = useRef(new Set<string>())
  const [runningSessionIds, setRunningSessionIds] = useState<
    ReadonlySet<string>
  >(() => new Set())
  const activeSessionIdRef = useRef(activeSessionId)
  activeSessionIdRef.current = activeSessionId

  const setSessionRunning = useCallback(
    (sessionId: string, running: boolean) => {
      if (runningRef.current.has(sessionId) === running) return
      if (running) {
        runningRef.current.add(sessionId)
      } else {
        runningRef.current.delete(sessionId)
      }
      setRunningSessionIds(new Set(runningRef.current))
    },
    []
  )

  const getTask = useCallback((sessionId: string) => {
    let task = tasksRef.current.get(sessionId)
    if (!task) {
      task = createGenerationTask()
      tasksRef.current.set(sessionId, task)
    }
    return task
  }, [])

  const discardPendingStreamUpdates = useCallback(
    (task: GenerationTask, generation: number) => {
      if (task.flushTimer !== null) {
        window.clearTimeout(task.flushTimer)
        task.flushTimer = null
      }
      task.pending = {
        generation,
        content: '',
        reasoning: '',
      }
    },
    []
  )

  const flushStreamUpdates = useCallback(
    (sessionId: string, task: GenerationTask, generation: number) => {
      if (generation !== task.generation) return
      if (task.flushTimer !== null) {
        window.clearTimeout(task.flushTimer)
        task.flushTimer = null
      }

      const pendingChunks = task.pending
      if (pendingChunks.generation !== generation) return
      if (!pendingChunks.reasoning && !pendingChunks.content) {
        return
      }

      task.pending = {
        generation,
        content: '',
        reasoning: '',
      }
      onMessageUpdate(sessionId, (prev) => {
        if (generation !== task.generation) return prev
        return updateLastAssistantMessage(prev, (message) => {
          let updatedMessage = message

          if (pendingChunks.reasoning) {
            updatedMessage = applyStreamingChunk(
              updatedMessage,
              'reasoning',
              pendingChunks.reasoning
            )
          }

          if (pendingChunks.content) {
            updatedMessage = applyStreamingChunk(
              updatedMessage,
              'content',
              pendingChunks.content
            )
          }

          return updatedMessage
        })
      })
    },
    [onMessageUpdate]
  )

  const scheduleStreamFlush = useCallback(
    (sessionId: string, task: GenerationTask, generation: number) => {
      if (generation !== task.generation) return
      if (task.flushTimer !== null) {
        return
      }

      task.flushTimer = window.setTimeout(() => {
        task.flushTimer = null
        flushStreamUpdates(sessionId, task, generation)
      }, STREAM_UPDATE_FLUSH_MS)
    },
    [flushStreamUpdates]
  )

  useEffect(
    () => () => {
      for (const task of tasksRef.current.values()) {
        task.generation += 1
        if (task.flushTimer !== null) {
          window.clearTimeout(task.flushTimer)
          task.flushTimer = null
        }
        task.stream?.dispose()
        task.abortController?.abort()
      }
      tasksRef.current.clear()
      runningRef.current.clear()
    },
    []
  )

  const getDisplayError = useCallback(
    (error: string) => {
      if (KNOWN_ERROR_MESSAGES.has(error)) {
        return t(error)
      }

      const connectionClosedSuffix = `: ${ERROR_MESSAGES.CONNECTION_CLOSED}`
      if (error.endsWith(connectionClosedSuffix)) {
        return `${error.slice(0, -ERROR_MESSAGES.CONNECTION_CLOSED.length)}${t(
          ERROR_MESSAGES.CONNECTION_CLOSED
        )}`
      }

      return error
    },
    [t]
  )

  const handleStreamUpdate = useCallback(
    (
      sessionId: string,
      task: GenerationTask,
      generation: number,
      type: 'reasoning' | 'content',
      chunk: string
    ) => {
      if (generation !== task.generation) return
      if (task.pending.generation !== generation) return
      task.pending[type] = mergePendingStreamChunk(task.pending[type], chunk)
      scheduleStreamFlush(sessionId, task, generation)
    },
    [scheduleStreamFlush]
  )

  const handleStreamComplete = useCallback(
    (sessionId: string, task: GenerationTask, generation: number) => {
      if (generation !== task.generation) return
      flushStreamUpdates(sessionId, task, generation)
      setSessionRunning(sessionId, false)
      onMessageUpdate(sessionId, (prev) => {
        if (generation !== task.generation) return prev
        return updateLastAssistantMessage(prev, (message) =>
          isAssistantMessageFinal(message)
            ? message
            : completeAssistantMessage(message)
        )
      })
    },
    [flushStreamUpdates, onMessageUpdate, setSessionRunning]
  )

  const handleStreamError = useCallback(
    (
      sessionId: string,
      task: GenerationTask,
      generation: number,
      error: string,
      errorCode?: string
    ) => {
      if (generation !== task.generation) return
      flushStreamUpdates(sessionId, task, generation)
      setSessionRunning(sessionId, false)
      const displayError = getDisplayError(error)
      handleServerError(new Error(displayError))
      const errorTitle = t(ERROR_MESSAGES.API_REQUEST_ERROR)
      onMessageUpdate(sessionId, (prev) => {
        if (generation !== task.generation) return prev
        return updateAssistantMessageWithError(
          prev,
          displayError,
          errorCode,
          errorTitle
        )
      })
    },
    [flushStreamUpdates, getDisplayError, onMessageUpdate, setSessionRunning, t]
  )

  const sendStreamingChat = useCallback(
    (sessionId: string, messages: Message[]) => {
      const task = getTask(sessionId)
      const generation = task.generation + 1
      task.generation = generation
      task.stream?.stop()
      discardPendingStreamUpdates(task, generation)
      if (!task.stream) {
        task.stream = createPlaygroundStreamRequest()
      }
      setSessionRunning(sessionId, true)
      const payload = buildChatCompletionPayload(
        messages,
        config,
        parameterEnabled,
        { imageModel }
      )
      void task.stream.send(payload, {
        onUpdate: (type, chunk) =>
          handleStreamUpdate(sessionId, task, generation, type, chunk),
        onComplete: () => handleStreamComplete(sessionId, task, generation),
        onError: (error, errorCode) =>
          handleStreamError(sessionId, task, generation, error, errorCode),
      })
    },
    [
      config,
      parameterEnabled,
      imageModel,
      getTask,
      setSessionRunning,
      discardPendingStreamUpdates,
      handleStreamUpdate,
      handleStreamComplete,
      handleStreamError,
    ]
  )

  const sendNonStreamingChat = useCallback(
    async (sessionId: string, messages: Message[]) => {
      const task = getTask(sessionId)
      const generation = task.generation + 1
      task.generation = generation
      task.stream?.stop()
      discardPendingStreamUpdates(task, generation)
      task.abortController?.abort()
      const abortController = new AbortController()
      task.abortController = abortController
      setSessionRunning(sessionId, true)

      const payload = buildChatCompletionPayload(
        messages,
        config,
        parameterEnabled,
        { imageModel }
      )

      try {
        const response = await sendChatCompletion(
          payload,
          abortController.signal
        )
        if (abortController.signal.aborted || task.generation !== generation) {
          return
        }

        if (!hasChatCompletionChoice(response)) {
          handleStreamError(
            sessionId,
            task,
            generation,
            ERROR_MESSAGES.API_REQUEST_ERROR
          )
          return
        }

        onMessageUpdate(sessionId, (prev) => {
          if (task.generation !== generation) return prev
          return updateLastAssistantMessage(prev, (message) => {
            const updatedMessage = applyChatCompletionResponse(
              message,
              response
            )

            return updatedMessage ?? message
          })
        })
      } catch (error: unknown) {
        if (abortController.signal.aborted || task.generation !== generation) {
          return
        }

        const { errorCode, errorMessage } = parseRequestErrorDetails(error)
        handleStreamError(sessionId, task, generation, errorMessage, errorCode)
      } finally {
        if (task.generation === generation) {
          task.abortController = null
          setSessionRunning(sessionId, false)
        }
      }
    },
    [
      config,
      parameterEnabled,
      imageModel,
      getTask,
      setSessionRunning,
      discardPendingStreamUpdates,
      onMessageUpdate,
      handleStreamError,
    ]
  )

  // Send in the active conversation; other conversations keep their own task.
  const sendChat = useCallback(
    (messages: Message[]) => {
      const sessionId = activeSessionIdRef.current
      if (!sessionId || runningRef.current.has(sessionId)) return
      if (config.stream) {
        sendStreamingChat(sessionId, messages)
      } else {
        void sendNonStreamingChat(sessionId, messages)
      }
    },
    [config.stream, sendStreamingChat, sendNonStreamingChat]
  )

  const stopGeneration = useCallback(
    (targetSessionId?: string) => {
      const sessionId = targetSessionId ?? activeSessionIdRef.current
      if (!sessionId) return
      const task = tasksRef.current.get(sessionId)
      if (!task) return

      const stoppedGeneration = task.generation
      flushStreamUpdates(sessionId, task, stoppedGeneration)
      const idleGeneration = stoppedGeneration + 1
      task.generation = idleGeneration
      discardPendingStreamUpdates(task, idleGeneration)
      task.stream?.stop()
      task.abortController?.abort()
      task.abortController = null
      setSessionRunning(sessionId, false)
      onMessageUpdate(sessionId, (prev) => {
        if (task.generation !== idleGeneration) return prev
        return updateLastAssistantMessage(prev, (message) =>
          isAssistantMessagePending(message)
            ? completeAssistantMessage(message)
            : message
        )
      })
    },
    [
      flushStreamUpdates,
      discardPendingStreamUpdates,
      onMessageUpdate,
      setSessionRunning,
    ]
  )

  const cancelSessionGeneration = useCallback(
    (sessionId: string) => {
      stopGeneration(sessionId)
      const task = tasksRef.current.get(sessionId)
      if (!task) return
      task.stream?.dispose()
      tasksRef.current.delete(sessionId)
    },
    [stopGeneration]
  )

  return {
    sendChat,
    stopGeneration,
    cancelSessionGeneration,
    runningSessionIds,
    isGenerating: activeSessionId
      ? runningSessionIds.has(activeSessionId)
      : false,
  }
}
