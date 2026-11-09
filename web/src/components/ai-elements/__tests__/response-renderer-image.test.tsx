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
import { render, screen } from '@testing-library/react'
import { sanitizeImageSrc, type ImageNode } from 'stream-markdown-parser'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'

import { ResponseImage } from '../response-renderer-image'

vi.mock('stream-markdown-parser', async (importOriginal) => {
  const actual = await importOriginal<typeof import('stream-markdown-parser')>()
  return {
    ...actual,
    sanitizeImageSrc: vi.fn(actual.sanitizeImageSrc),
  }
})

const sanitizeMock = vi.mocked(sanitizeImageSrc)

function imageNode(src: string, alt = 'generated image'): ImageNode {
  return { type: 'image', src, alt, title: null, raw: '' }
}

beforeEach(() => {
  sanitizeMock.mockClear()
})

afterEach(() => {
  vi.restoreAllMocks()
})

describe('ResponseImage', () => {
  test('renders inline base64 images without scanning the whole URL', () => {
    const src = `data:image/jpeg;base64,${'A'.repeat(1_000_000)}`

    render(<ResponseImage node={imageNode(src)} />)

    expect(screen.getByRole('img').getAttribute('src')).toBe(src)
    expect(sanitizeMock).not.toHaveBeenCalled()
  })

  test('keeps validating inline images whose type is not renderable', () => {
    render(
      <ResponseImage
        node={imageNode('data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=')}
      />
    )

    expect(screen.queryByRole('img')).toBeNull()
    expect(sanitizeMock).toHaveBeenCalledTimes(1)
  })

  test('still validates regular URLs and only once per src', () => {
    const node = imageNode('https://example.com/cat.png')
    const { rerender } = render(<ResponseImage node={node} />)

    rerender(<ResponseImage node={{ ...node }} />)

    expect(screen.getByRole('img').getAttribute('src')).toBe(node.src)
    expect(sanitizeMock).toHaveBeenCalledTimes(1)
  })
})
