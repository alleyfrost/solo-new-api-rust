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
import type { Message } from '../../types'
import {
  extractInlineImageUrls,
  removeInlineImages,
} from '../message/message-images'
import { getMessageContent } from '../message/message-utils'

export const MAX_HISTORY_TURNS = 3
export const MAX_HISTORY_MESSAGE_CHARS = 300
export const MAX_HISTORY_CHARS = 2000
export const MAX_HISTORY_IMAGES = 2
export const MAX_CURRENT_IMAGES = 2
/** The image API only accepts prompt + references; keep the request body bounded. */
export const MAX_REFERENCE_CHARS = 18_000_000

export type ImageEditContext = {
  prompt: string
  imageUrls: string[]
}

function isSupportedImageUrl(url: string): boolean {
  return (
    url.startsWith('data:image/') ||
    url.startsWith('http://') ||
    url.startsWith('https://')
  )
}

function imagesOf(message: Message): string[] {
  return extractInlineImageUrls(getMessageContent(message)).filter(
    isSupportedImageUrl
  )
}

function textOf(message: Message): string {
  const text = removeInlineImages(getMessageContent(message)).trim()
  return text.length > MAX_HISTORY_MESSAGE_CHARS
    ? `${text.slice(0, MAX_HISTORY_MESSAGE_CHARS)}…`
    : text
}

/** Keep the last few turns, at most MAX_HISTORY_TURNS user messages. */
function recentTurns(history: Message[]): Message[] {
  const selected: Message[] = []
  let users = 0
  for (let index = history.length - 1; index >= 0; index--) {
    const message = history[index]
    if (message.from === 'system') {
      continue
    }
    if (message.from === 'user') {
      users += 1
      if (users > MAX_HISTORY_TURNS) {
        break
      }
    }
    selected.unshift(message)
  }
  return selected
}

function referenceLabel(index: number, historyCount: number): string {
  return index < historyCount ? '历史图片' : '用户上传的图片'
}

/**
 * Build the prompt for an image model that should continue from previous
 * turns. The image endpoint has no conversation concept, so history travels
 * as labelled text and the referenced pictures are numbered in order.
 */
export function buildImageEditContext(
  history: Message[],
  currentText: string,
  currentImages: string[] = []
): ImageEditContext {
  const instruction = removeInlineImages(currentText).trim()
  const selected = recentTurns(history)
  const historyImages: string[] = []
  for (const message of selected) {
    historyImages.push(...imagesOf(message))
  }
  const current = currentImages
    .filter(isSupportedImageUrl)
    .slice(0, MAX_CURRENT_IMAGES)
  const ordered = [...historyImages.slice(-MAX_HISTORY_IMAGES), ...current]
  let totalChars = ordered.reduce((size, url) => size + url.length, 0)
  while (ordered.length > current.length && totalChars > MAX_REFERENCE_CHARS) {
    totalChars -= ordered.shift()?.length ?? 0
  }

  if (ordered.length === 0) {
    return { prompt: instruction, imageUrls: [] }
  }

  const historyCount = ordered.length - current.length
  type Line = { role: string; text: string; refs: number[] }
  const lines: Line[] = []
  for (const message of selected) {
    const text = textOf(message)
    const refs = imagesOf(message)
      .map((url) => ordered.indexOf(url) + 1)
      .filter((index) => index > 0)
    if (!text && refs.length === 0) {
      continue
    }
    lines.push({
      role: message.from === 'assistant' ? '助手' : '用户',
      text,
      refs,
    })
  }

  const rendered: string[] = []
  let used = 0
  for (let index = lines.length - 1; index >= 0; index--) {
    const line = lines[index]
    const kind = line.role === '助手' ? '生成图片' : '上传图片'
    const refs =
      line.refs.length > 0
        ? `（${kind}，见参考图 ${line.refs.join('、')}）`
        : ''
    const body = line.text ? `${line.text}${refs}` : refs
    const renderedLine = `${line.role}：${body}`
    if (rendered.length > 0 && used + renderedLine.length > MAX_HISTORY_CHARS) {
      break
    }
    used += renderedLine.length
    rendered.unshift(renderedLine)
  }

  const sections: string[] = []
  if (rendered.length > 0) {
    sections.push(`【历史对话】\n${rendered.join('\n')}`)
  }
  sections.push(
    `【参考图】\n${ordered
      .map(
        (_, index) =>
          `参考图 ${index + 1}：${referenceLabel(index, historyCount)}`
      )
      .join('\n')}`
  )
  sections.push(`【当前指令】\n${instruction}`)

  return { prompt: sections.join('\n'), imageUrls: ordered }
}
