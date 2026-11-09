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

import type { Message } from '../../../types'
import { OMITTED_IMAGE_MARKER } from '../message-images'
import { formatMessageForAPI } from '../message-utils'

test('history sent upstream carries the image marker instead of the payload', () => {
  const message: Message = {
    key: 'image-chat',
    from: 'assistant',
    versions: [
      {
        id: 'image-version',
        content: `生成完成\n\n![image](data:image/png;base64,${'A'.repeat(2048)})`,
      },
    ],
  }

  const payload = formatMessageForAPI(message)

  expect(payload.content).toBe(`生成完成\n\n${OMITTED_IMAGE_MARKER}`)
})

test('the latest user message sends its image as a content part', () => {
  const dataUrl = `data:image/png;base64,${'A'.repeat(64)}`
  const message: Message = {
    key: 'edit-request',
    from: 'user',
    versions: [
      {
        id: 'edit-version',
        content: `把背景换成雪山\n\n![photo.png](${dataUrl})`,
      },
    ],
  }

  const payload = formatMessageForAPI(message, { includeImages: true })

  expect(payload.content).toEqual([
    { type: 'text', text: '把背景换成雪山' },
    { type: 'image_url', image_url: { url: dataUrl } },
  ])
})

test('an earlier user message keeps the image marker even when asked to include images', () => {
  const message: Message = {
    key: 'old-request',
    from: 'assistant',
    versions: [
      {
        id: 'old-version',
        content: `旧回复\n\n![image](data:image/png;base64,QUJD)`,
      },
    ],
  }

  const payload = formatMessageForAPI(message, { includeImages: true })

  expect(payload.content).toBe(`旧回复\n\n${OMITTED_IMAGE_MARKER}`)
})
