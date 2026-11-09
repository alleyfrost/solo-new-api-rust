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
import 'fake-indexeddb/auto'
import { expect, test } from 'vitest'

import type { Message } from '../../../types'
import { OMITTED_IMAGE_MARKER } from '../../message/message-images'
import {
  cacheSessionImages,
  deleteSessionImages,
  MAX_CACHED_IMAGE_MESSAGES,
  rehydrateSessionImages,
} from '../image-cache'

const DATA_URL = 'data:image/png;base64,QUJD'

function imageMessage(key: string, count = 1): Message {
  const markdown = Array.from(
    { length: count },
    () => `![image](${DATA_URL})`
  ).join('\n')
  return {
    key,
    from: 'assistant',
    versions: [{ id: `${key}-v`, content: markdown }],
  }
}

function storedMessage(key: string, count = 1): Message {
  return {
    key,
    from: 'assistant',
    versions: [
      {
        id: `${key}-v`,
        content: Array.from({ length: count }, () => OMITTED_IMAGE_MARKER).join(
          '\n'
        ),
      },
    ],
  }
}

test('stored markers are rehydrated from IndexedDB in order', async () => {
  const userId = 101
  const sessionId = 'roundtrip'
  await cacheSessionImages(userId, sessionId, [imageMessage('m1', 2)])

  const restored = await rehydrateSessionImages(userId, sessionId, [
    storedMessage('m1', 2),
  ])

  expect(restored[0].versions[0].content).toBe(
    `![image](${DATA_URL})\n![image](${DATA_URL})`
  )
})

test('only the newest image-bearing messages keep their blobs', async () => {
  const userId = 102
  const sessionId = 'eviction'
  const count = MAX_CACHED_IMAGE_MESSAGES + 2
  const messages = Array.from({ length: count }, (_, index) =>
    imageMessage(`m${index}`)
  )
  await cacheSessionImages(userId, sessionId, messages)

  const restored = await rehydrateSessionImages(
    userId,
    sessionId,
    messages.map((_, index) => storedMessage(`m${index}`))
  )

  const texts = restored.map((message) => message.versions[0].content)
  expect(texts.slice(0, 2)).toEqual([
    OMITTED_IMAGE_MARKER,
    OMITTED_IMAGE_MARKER,
  ])
  expect(texts.slice(2)).toEqual(
    Array.from(
      { length: MAX_CACHED_IMAGE_MESSAGES },
      () => `![image](${DATA_URL})`
    )
  )
})

test('deleting a session drops its cached images', async () => {
  const userId = 103
  const sessionId = 'deleted'
  await cacheSessionImages(userId, sessionId, [imageMessage('m1')])
  await deleteSessionImages(userId, sessionId)

  const restored = await rehydrateSessionImages(userId, sessionId, [
    storedMessage('m1'),
  ])

  expect(restored[0].versions[0].content).toBe(OMITTED_IMAGE_MARKER)
})
