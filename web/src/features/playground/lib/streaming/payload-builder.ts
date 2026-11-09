import { MESSAGE_ROLES } from '../../constants'
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
import type {
  ChatCompletionMessage,
  ChatCompletionRequest,
  ContentPart,
  Message,
  PlaygroundConfig,
  ParameterEnabled,
} from '../../types'
import {
  extractInlineImageUrls,
  removeInlineImages,
} from '../message/message-images'
import {
  formatMessageForAPI,
  getMessageContent,
  isValidMessage,
} from '../message/message-utils'
import { buildImageEditContext } from './image-edit-context'

export type BuildPayloadOptions = {
  imageModel?: boolean
}

function withImages(text: string, imageUrls: string[]): string | ContentPart[] {
  if (imageUrls.length === 0) {
    return text
  }
  const parts: ContentPart[] = []
  if (text) {
    parts.push({ type: 'text', text })
  }
  for (const url of imageUrls) {
    parts.push({ type: 'image_url', image_url: { url } })
  }
  return parts
}

/**
 * Build API request payload from messages and config
 */
export function buildChatCompletionPayload(
  messages: Message[],
  config: PlaygroundConfig,
  parameterEnabled: ParameterEnabled,
  options: BuildPayloadOptions = {}
): ChatCompletionRequest {
  const validMessages = messages.filter(isValidMessage)
  const latestUserIndex = validMessages.reduce(
    (latest, message, index) =>
      message.from === MESSAGE_ROLES.USER ? index : latest,
    -1
  )

  let processedMessages: ChatCompletionMessage[]
  let imageModel = false
  if (options.imageModel && latestUserIndex !== -1) {
    const latest = validMessages[latestUserIndex]
    const content = getMessageContent(latest)
    const currentText = removeInlineImages(content).trim()
    const currentImages = extractInlineImageUrls(content)
    const context = buildImageEditContext(
      validMessages.slice(0, latestUserIndex),
      currentText,
      currentImages
    )
    // The image bridge reads only the last user message; history already
    // travels inside the assembled prompt, so nothing else is sent.
    processedMessages = [
      {
        role: MESSAGE_ROLES.USER,
        content: withImages(context.prompt, context.imageUrls),
      },
    ]
    imageModel = true
  } else {
    processedMessages = validMessages.map((message, index) =>
      formatMessageForAPI(message, { includeImages: index === latestUserIndex })
    )
  }

  const payload: ChatCompletionRequest = {
    model: config.model,
    group: config.group,
    messages: processedMessages,
    stream: config.stream,
  }

  if (imageModel) {
    payload.response_format = 'b64_json'
  }

  if (parameterEnabled.temperature) {
    payload.temperature = config.temperature
  }

  if (parameterEnabled.top_p) {
    payload.top_p = config.top_p
  }

  if (parameterEnabled.max_tokens) {
    payload.max_tokens = config.max_tokens
  }

  if (parameterEnabled.frequency_penalty) {
    payload.frequency_penalty = config.frequency_penalty
  }

  if (parameterEnabled.presence_penalty) {
    payload.presence_penalty = config.presence_penalty
  }

  if (parameterEnabled.seed && config.seed !== null) {
    payload.seed = config.seed
  }

  return payload
}
