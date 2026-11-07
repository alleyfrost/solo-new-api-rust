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

import {
  OMITTED_IMAGE_MARKER,
  splitMessageImages,
  stripMessageImagesForStorage,
  stripOmittedImageMarkers,
} from '../message-images'

const DATA_URL = 'data:image/png;base64,QUJD'

test('splits inline image markdown out of the surrounding text', () => {
  const segments = splitMessageImages(
    `画好了\n\n![image](${DATA_URL})\n\n需要调整吗`
  )

  expect(segments).toEqual([
    { kind: 'text', text: '画好了\n\n' },
    { kind: 'image', alt: 'image', src: DATA_URL },
    { kind: 'text', text: '\n\n需要调整吗' },
  ])
})

test('keeps several images in one message', () => {
  const segments = splitMessageImages(
    `![a](${DATA_URL})![b](${DATA_URL.replace('QUJD', 'REVG')})`
  )

  expect(segments).toHaveLength(2)
  expect(segments.map((segment) => segment.kind)).toEqual(['image', 'image'])
})

test('a partially streamed image shows a placeholder instead of raw base64', () => {
  const segments = splitMessageImages(
    `画好了\n\n![image](${DATA_URL.slice(0, -2)}`
  )

  expect(segments).toEqual([
    { kind: 'text', text: '画好了\n\n' },
    { kind: 'pending-image' },
  ])
})

test('the persisted marker renders as an omitted-image note', () => {
  const segments = splitMessageImages(`before ${OMITTED_IMAGE_MARKER} after`)

  expect(segments.map((segment) => segment.kind)).toEqual([
    'text',
    'omitted-image',
    'text',
  ])
})

test('storage strips image payloads but keeps the marker', () => {
  const stored = stripMessageImagesForStorage(
    `图：![image](${DATA_URL})\n\n以及 ![cat](${DATA_URL})`
  )

  expect(stored).toBe(
    `图：${OMITTED_IMAGE_MARKER}\n\n以及 ${OMITTED_IMAGE_MARKER}`
  )
})

test('a payload built from stored history drops the markers', () => {
  expect(stripOmittedImageMarkers(`before ${OMITTED_IMAGE_MARKER} after`)).toBe(
    'before  after'
  )
  expect(stripOmittedImageMarkers(` ${OMITTED_IMAGE_MARKER} `)).toBe('')
})
