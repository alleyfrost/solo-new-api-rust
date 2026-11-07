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
/**
 * Generated images come back inline as base64 markdown, far beyond the
 * markdown parser's size budget, so they are split out before rendering.
 * Data URLs are replaced with this marker when persisting history: a few
 * megabytes of base64 would blow past the localStorage quota.
 */
export const OMITTED_IMAGE_MARKER = '[[image-omitted]]'

export type MessageImageSegment =
  | { kind: 'text'; text: string }
  | { kind: 'image'; alt: string; src: string }
  | { kind: 'pending-image' }
  | { kind: 'omitted-image' }

const DATA_IMAGE_PATTERN = /!\[([^\]]*)\]\((data:image\/[^)\s]+)\)/g
const MARKER_PATTERN = /\[\[image-omitted\]\]/g
const SEGMENT_PATTERN =
  /\[\[image-omitted\]\]|!\[([^\]]*)\]\((data:image\/[^)\s]+)\)/g

function pushText(segments: MessageImageSegment[], text: string) {
  if (text) {
    segments.push({ kind: 'text', text })
  }
}

function pushPendingTail(segments: MessageImageSegment[], tail: string) {
  const start = tail.indexOf('![')
  const open = start === -1 ? -1 : tail.indexOf('](data:image', start)
  const incomplete = open !== -1 && !tail.slice(open).includes(')')

  if (incomplete) {
    pushText(segments, tail.slice(0, start))
    segments.push({ kind: 'pending-image' })
    return
  }

  pushText(segments, tail)
}

export function splitMessageImages(content: string): MessageImageSegment[] {
  const segments: MessageImageSegment[] = []
  SEGMENT_PATTERN.lastIndex = 0
  let cursor = 0
  let match: RegExpExecArray | null

  while ((match = SEGMENT_PATTERN.exec(content)) !== null) {
    pushText(segments, content.slice(cursor, match.index))

    if (match[0] === OMITTED_IMAGE_MARKER) {
      segments.push({ kind: 'omitted-image' })
    } else {
      segments.push({
        kind: 'image',
        alt: match[1] ?? '',
        src: match[2] ?? '',
      })
    }

    cursor = match.index + match[0].length
  }

  pushPendingTail(segments, content.slice(cursor))
  return segments
}

export function stripMessageImagesForStorage(content: string): string {
  DATA_IMAGE_PATTERN.lastIndex = 0
  return content.replace(DATA_IMAGE_PATTERN, OMITTED_IMAGE_MARKER)
}

export function stripOmittedImageMarkers(content: string): string {
  MARKER_PATTERN.lastIndex = 0
  return content.replace(MARKER_PATTERN, '').trim()
}
