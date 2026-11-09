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
import { expect, test } from 'vitest'

import type {
  Message,
  ParameterEnabled,
  PlaygroundConfig,
} from '../../../types'
import { buildChatCompletionPayload } from '../payload-builder'

const DATA_URL = 'data:image/png;base64,QUJD'

const config: PlaygroundConfig = {
  model: 'img-model',
  group: 'default',
  temperature: 1,
  top_p: 1,
  max_tokens: 1024,
  frequency_penalty: 0,
  presence_penalty: 0,
  seed: null,
  stream: false,
}

const parameterEnabled: ParameterEnabled = {
  temperature: false,
  top_p: false,
  max_tokens: false,
  frequency_penalty: false,
  presence_penalty: false,
  seed: false,
}

function userMessage(key: string, content: string): Message {
  return { key, from: 'user', versions: [{ id: `${key}-v`, content }] }
}

function assistantMessage(key: string, content: string): Message {
  return { key, from: 'assistant', versions: [{ id: `${key}-v`, content }] }
}

test('only the latest user message carries its image upstream', () => {
  const messages = [
    userMessage('first', `第一张\n\n![image](${DATA_URL})`),
    assistantMessage('reply', '好的'),
    userMessage('second', `改背景\n\n![image](${DATA_URL})`),
  ]

  const payload = buildChatCompletionPayload(
    messages,
    config,
    parameterEnabled
  )

  expect(payload.messages[0].content).toBe('第一张\n\n[[image-omitted]]')
  expect(payload.messages[2].content).toEqual([
    { type: 'text', text: '改背景' },
    { type: 'image_url', image_url: { url: DATA_URL } },
  ])
})

test('text-only conversations keep the plain string content', () => {
  const payload = buildChatCompletionPayload(
    [userMessage('plain', '你好')],
    config,
    parameterEnabled
  )

  expect(payload.messages).toEqual([{ role: 'user', content: '你好' }])
})
