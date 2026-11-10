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
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, renderHook, waitFor } from '@testing-library/react'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useTopNavLinks } from '../use-top-nav-links'

function wrapperWith(status: Record<string, unknown>) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  client.setQueryData(['status'], status)
  function Wrapper(props: { children: ReactNode }) {
    return (
      <QueryClientProvider client={client}>
        {props.children}
      </QueryClientProvider>
    )
  }
  return Wrapper
}

describe('useTopNavLinks', () => {
  beforeEach(() => {
    vi.stubGlobal('localStorage', {
      getItem: () => null,
      setItem: () => undefined,
      removeItem: () => undefined,
    })
  })

  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
  })

  it('no longer exposes a console link even when HeaderNavModules enables it', async () => {
    const { result } = renderHook(() => useTopNavLinks(), {
      wrapper: wrapperWith({
        HeaderNavModules: JSON.stringify({ console: true }),
      }),
    })
    await waitFor(() => expect(result.current.length).toBeGreaterThan(0))
    expect(
      result.current.find((link) => link.href === '/dashboard')
    ).toBeUndefined()
  })

  it('keeps home, docs and about links', async () => {
    const { result } = renderHook(() => useTopNavLinks(), {
      wrapper: wrapperWith({}),
    })
    await waitFor(() => expect(result.current.length).toBeGreaterThan(0))
    const hrefs = result.current.map((link) => link.href)
    expect(hrefs).toContain('/')
    expect(hrefs).toContain('/docs')
    expect(hrefs).toContain('/about')
  })
})
