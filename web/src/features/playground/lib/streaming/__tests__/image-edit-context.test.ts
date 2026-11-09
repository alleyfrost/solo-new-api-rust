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
import {
  buildImageEditContext,
  MAX_HISTORY_MESSAGE_CHARS,
} from '../image-edit-context'

const IMAGE_1 = 'data:image/png;base64,QUJD'
const IMAGE_2 = 'data:image/png;base64,REVG'

function user(key: string, content: string): Message {
  return { key, from: 'user', versions: [{ id: `${key}-v`, content }] }
}

function assistant(key: string, content: string): Message {
  return { key, from: 'assistant', versions: [{ id: `${key}-v`, content }] }
}

test('history text and numbered references travel in one labelled prompt', () => {
  const context = buildImageEditContext(
    [
      user('u1', '画一只猫'),
      assistant('a1', `![image](${IMAGE_1})`),
      user('u2', '把背景换成雪山'),
      assistant('a2', '完成'),
    ],
    '再把天空换成黄昏',
    [IMAGE_2]
  )

  expect(context.prompt).toBe(
    [
      '【历史对话】',
      '用户：画一只猫',
      '助手：（生成图片，见参考图 1）',
      '用户：把背景换成雪山',
      '助手：完成',
      '【参考图】',
      '参考图 1：历史图片',
      '参考图 2：用户上传的图片',
      '【当前指令】',
      '再把天空换成黄昏',
    ].join('\n')
  )
  expect(context.imageUrls).toEqual([IMAGE_1, IMAGE_2])
})

test('without references the history is not mixed into the prompt', () => {
  const context = buildImageEditContext(
    [user('u1', '画一只猫'), assistant('a1', '好了')],
    '画一只狗'
  )

  expect(context).toEqual({ prompt: '画一只狗', imageUrls: [] })
})

test('keeps only the two most recent history images', () => {
  const context = buildImageEditContext(
    [
      user('u1', '第一张'),
      assistant('a1', `![image](${IMAGE_1})`),
      user('u2', '第二张'),
      assistant('a2', `![image](${IMAGE_2})`),
      user('u3', '第三张'),
      assistant('a3', `![image](data:image/png;base64,R0hJ)`),
    ],
    '继续'
  )

  expect(context.imageUrls).toEqual([IMAGE_2, 'data:image/png;base64,R0hJ'])
  expect(context.prompt).toContain('参考图 1：历史图片')
  expect(context.prompt).toContain('参考图 2：历史图片')
})

test('only the last three user turns are described', () => {
  const context = buildImageEditContext(
    [
      user('u1', '第一轮'),
      assistant('a1', '好'),
      user('u2', '第二轮'),
      assistant('a2', '好'),
      user('u3', '第三轮'),
      assistant('a3', `![image](${IMAGE_1})`),
      user('u4', '第四轮'),
      assistant('a4', '好'),
    ],
    '继续改'
  )

  expect(context.prompt).not.toContain('第一轮')
  expect(context.prompt).toContain('第二轮')
  expect(context.prompt).toContain('第四轮')
})

test('long history messages are truncated', () => {
  const context = buildImageEditContext(
    [
      user('u1', '一'.repeat(MAX_HISTORY_MESSAGE_CHARS + 50)),
      assistant('a1', `![image](${IMAGE_1})`),
    ],
    '继续'
  )

  expect(context.prompt).toContain('…')
  const historyLine = context.prompt.split('\n')[1]
  expect(historyLine.length).toBeLessThanOrEqual(
    MAX_HISTORY_MESSAGE_CHARS + '用户：'.length + 1
  )
})
