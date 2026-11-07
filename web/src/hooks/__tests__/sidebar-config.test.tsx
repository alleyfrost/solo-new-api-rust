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
import {
  cleanup,
  renderHook,
} from '@testing-library/react'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { useAuthStore } from '@/stores/auth-store'

import { useSidebarConfig } from '../use-sidebar-config'
import { useSidebarData } from '../use-sidebar-data'

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
  useAuthStore.getState().auth.reset()
})

function sidebarFor(admin?: object, user?: object, canConfigure = true) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  client.setQueryData(['status'], {
    SidebarModulesAdmin: admin ? JSON.stringify(admin) : '',
  })
  useAuthStore.getState().auth.setUser({
    id: 1,
    username: 'alice',
    role: 1,
    permissions: { sidebar_settings: canConfigure },
    sidebar_modules: user ? JSON.stringify(user) : '',
  })
  function Wrapper(props: { children: ReactNode }) {
    return (
      <QueryClientProvider client={client}>
        {props.children}
      </QueryClientProvider>
    )
  }
  return renderHook(() => useSidebarConfig(useSidebarData().navGroups), {
    wrapper: Wrapper,
  })
}

function titlesOf(result: { current: ReturnType<typeof useSidebarData>['navGroups'] }) {
  return result.current.flatMap((group) => group.items).map((item) => item.title)
}

describe('sidebar visibility with current routes', () => {
  it('shows all product entries by default (empty admin config falls back to defaults)', () => {
    const { result } = sidebarFor()
    // Playground leads its own group, mirroring the upstream layout.
    expect(titlesOf(result)).toEqual([
      'Playground',
      'Overview',
      'Dashboard',
      'API Keys',
      'Usage Logs',
      'Channels',
      'Models',
      'System Settings',
    ])
  })

  it('legacy configurations missing module keys keep the entries visible', () => {
    const { result } = sidebarFor({ console: { enabled: true } })
    const titles = titlesOf(result)
    expect(titles).toContain('Usage Logs')
    expect(titles).toContain('API Keys')
    expect(titles).toContain('Playground')
  })

  it.each([
    [{ console: { enabled: true, log: false } }, undefined],
    [{ console: { enabled: false } }, { console: { log: true } }],
    [undefined, { console: { enabled: true, log: false } }],
    [undefined, { console: { enabled: false } }],
  ])(
    'admin or user disablement hides Usage Logs (%j, %j)',
    (admin, user) => {
      const { result } = sidebarFor(admin, user)
      const titles = titlesOf(result)
      expect(titles).not.toContain('Usage Logs')
      // Entries outside the console section are untouched by its toggles.
      expect(titles).toContain('Playground')
    }
  )

  it('admin section disablement hides Channels and Models for members', () => {
    const { result } = sidebarFor({ admin: { enabled: false } })
    const titles = titlesOf(result)
    expect(titles).not.toContain('Channels')
    expect(titles).not.toContain('Models')
    expect(titles).toContain('Overview')
    // Unmapped URLs fall back to visible, so settings is never locked away.
    expect(titles).toContain('System Settings')
  })

  it('playground toggle hides the Playground entry only', () => {
    const { result } = sidebarFor({ chat: { enabled: true, playground: false } })
    const titles = titlesOf(result)
    expect(titles).not.toContain('Playground')
    expect(titles).toContain('Overview')
    expect(titles).toContain('API Keys')
  })

  it('users without sidebar configuration permission retain the admin view', () => {
    const { result } = sidebarFor(undefined, { console: { log: false } }, false)
    expect(titlesOf(result)).toContain('Usage Logs')
  })
})
