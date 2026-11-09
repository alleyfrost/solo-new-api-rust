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

import { appendImageAttachments } from '../input-control-utils'

test('appends attached images to the message as inline markdown', () => {
  const text = appendImageAttachments('把背景换成雪山', [
    {
      url: 'data:image/png;base64,QUJD',
      mediaType: 'image/png',
      filename: 'photo.png',
    },
  ])

  expect(text).toBe(
    '把背景换成雪山\n\n![photo.png](data:image/png;base64,QUJD)'
  )
})

test('leaves the text untouched when no image is attached', () => {
  expect(
    appendImageAttachments('你好', [
      { url: 'blob:mock', mediaType: 'application/pdf', filename: 'a.pdf' },
    ])
  ).toBe('你好')
  expect(appendImageAttachments('你好')).toBe('你好')
})

test('keeps multiple reference images in order', () => {
  const text = appendImageAttachments('融合两张图', [
    { url: 'data:image/png;base64,QQ==', mediaType: 'image/png' },
    { url: 'data:image/jpeg;base64,Qg==', mediaType: 'image/jpeg' },
  ])

  expect(text).toBe(
    '融合两张图\n\n![image](data:image/png;base64,QQ==)\n![image](data:image/jpeg;base64,Qg==)'
  )
})
