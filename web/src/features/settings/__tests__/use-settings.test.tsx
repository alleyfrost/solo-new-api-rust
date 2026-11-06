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
import { afterEach, describe, expect, test, vi } from 'vitest'

import { useOptions } from '../use-settings'

/**
 * Settings cards seed local form state from `values` in a `useEffect` that
 * depends on it. A fresh object on every render makes that effect re-run on
 * every commit, so typing is reverted instantly and the page never settles.
 */

const control = vi.hoisted(() => ({ gate: null as null | Promise<void> }))

vi.mock('../api', () => ({
  getOptions: async () => {
    if (control.gate) await control.gate
    return {
      success: true,
      data: [
        { key: 'SystemName', value: 'Solo' },
        { key: 'Notice', value: 'hello' },
      ],
    }
  },
  updateOption: vi.fn(),
  cleanupLogsBefore: vi.fn(),
}))

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    )
  }
}

describe('useOptions().values', () => {
  afterEach(() => {
    control.gate = null
    cleanup()
  })

  test('maps option rows into a keyed record', async () => {
    const { result } = renderHook(() => useOptions(), {
      wrapper: createWrapper(),
    })

    await waitFor(() => expect(result.current.isLoading).toBe(false))

    expect(result.current.values).toEqual({
      SystemName: 'Solo',
      Notice: 'hello',
    })
  })

  test('keeps one reference across re-renders once loaded', async () => {
    const { rerender, result } = renderHook(() => useOptions(), {
      wrapper: createWrapper(),
    })

    await waitFor(() => expect(result.current.isLoading).toBe(false))
    const first = result.current.values
    rerender()
    rerender()

    expect(result.current.values).toBe(first)
  })

  test('keeps one reference across re-renders while loading', async () => {
    let release: () => void = () => {}
    control.gate = new Promise<void>((resolve) => {
      release = resolve
    })
    const { rerender, result } = renderHook(() => useOptions(), {
      wrapper: createWrapper(),
    })

    const first = result.current.values
    rerender()
    expect(result.current.isLoading).toBe(true)
    expect(result.current.values).toBe(first)

    release()
    await waitFor(() => expect(result.current.isLoading).toBe(false))
  })
})
