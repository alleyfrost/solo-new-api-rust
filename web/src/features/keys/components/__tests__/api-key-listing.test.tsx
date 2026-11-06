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
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from '@tanstack/react-router'
import {
  act,
  cleanup,
  render,
  screen,
  within,
  waitFor,
} from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { createInstance } from 'i18next'
import { I18nextProvider } from 'react-i18next'
import { Toaster, toast } from 'sonner'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import zh from '@/i18n/locales/zh.json'
import { api } from '@/lib/api'
import { apiKeySchema, type ApiKey } from '../../types'
import { ApiKeysProvider } from '../api-keys-provider'
import { ApiKeysTable } from '../api-keys-table'

const key = apiKeySchema.parse({
  id: 7,
  name: 'production',
  key: 'demo********1234',
  status: 1,
  remain_quota: 0,
  used_quota: 0,
  unlimited_quota: true,
  expired_time: -1,
  created_time: 0,
  accessed_time: 0,
  group: 'default',
  model_limits_enabled: false,
})
const i18n = createInstance()
await i18n.init({
  lng: 'en',
  resources: { en: { translation: {} } },
  initAsync: false,
})
const clients: QueryClient[] = []

beforeEach(() => {
  vi.spyOn(window, 'scrollTo').mockImplementation(() => {})
  localStorage.clear()
  vi.spyOn(api, 'get').mockResolvedValue({ data: { success: true, data: {} } })
})
afterEach(() => {
  cleanup()
  toast.dismiss()
  localStorage.clear()
  clients.splice(0).forEach((client) => client.clear())
})

function KeysPage() {
  return (
    <ApiKeysProvider>
      <ApiKeysTable />
      <Toaster />
    </ApiKeysProvider>
  )
}

async function renderKeysPage(status = 1, overrides: Partial<ApiKey> = {}) {
  let currentKey = { ...key, status, ...overrides }
  vi.mocked(api.get).mockImplementation(async (url) => {
    if (url.startsWith('/api/token/')) {
      return {
        data: { success: true, data: { items: [currentKey], total: 1 } },
      }
    }
    return { data: { success: true, data: { default: { ratio: 1 } } } }
  })
  const post = vi.spyOn(api, 'post').mockResolvedValue({
    data: { success: true, data: { key: 'fake-key-for-test-only' } },
  })
  const put = vi.spyOn(api, 'put').mockImplementation(async (_url, data) => {
    const update = data as { id: number; status: number }
    currentKey = { ...currentKey, status: update.status }
    return { data: { success: true, data: currentKey } }
  })
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  client.setQueryData(['status'], {})
  clients.push(client)
  const root = createRootRoute()
  const auth = createRoute({ getParentRoute: () => root, id: '_authenticated' })
  const keysRoute = createRoute({
    getParentRoute: () => auth,
    path: 'keys/',
    component: KeysPage,
  })
  const router = createRouter({
    routeTree: root.addChildren([auth.addChildren([keysRoute])]),
    history: createMemoryHistory({ initialEntries: ['/keys/'] }),
  })
  await router.load()
  render(
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={client}>
        <RouterProvider router={router} />
      </QueryClientProvider>
    </I18nextProvider>
  )
  await screen.findByText(currentKey.name)
  return { post, put }
}

it('combines creation and last use while keeping expiry, models and IP restrictions separate', async () => {
  await renderKeysPage()
  for (const name of ['Name', 'API Key', 'Group', 'Models', 'IP Restriction']) {
    expect(screen.getByRole('columnheader', { name })).toBeInTheDocument()
  }
  expect(screen.getByRole('columnheader', { name: 'Time' })).toBeInTheDocument()
  expect(
    screen.getByRole('columnheader', { name: 'Expires' })
  ).toBeInTheDocument()
  const timeCell = screen.getByRole('cell', { name: /Created.*Last Used/ })
  expect(within(timeCell).getByText('Last Used')).toBeInTheDocument()
  // 只记用量不记金额：不再有额度/金额列。
  expect(
    screen.queryByRole('columnheader', { name: /Quota|Amount/ })
  ).not.toBeInTheDocument()
})

it('restores dates hidden by the old default and preserves unrelated column preferences', async () => {
  localStorage.setItem(
    'api-keys:column-visibility',
    JSON.stringify({
      created_time: false,
      accessed_time: false,
      expired_time: false,
      model_limits: false,
    })
  )
  await renderKeysPage()
  expect(screen.getByRole('columnheader', { name: 'Time' })).toBeInTheDocument()
  expect(
    screen.getByRole('columnheader', { name: 'Expires' })
  ).toBeInTheDocument()
  expect(
    screen.queryByRole('columnheader', { name: 'Models' })
  ).not.toBeInTheDocument()
})

it.each([
  [1, 'Disable', 2, 'Disabled'],
  [2, 'Enable', 1, 'Enabled'],
])(
  'keeps status %s toggling at its original row button without fetching a full key',
  async (status, action, nextStatus, nextLabel) => {
    const { post, put } = await renderKeysPage(status)
    const user = userEvent.setup()
    const button = screen.getByRole('button', { name: action })
    act(() => button.focus())
    await user.keyboard('{Enter}')
    await waitFor(() =>
      expect(put).toHaveBeenCalledWith('/api/token/?status_only=true', {
        id: 7,
        status: nextStatus,
      })
    )
    await screen.findByText(nextLabel)
    expect(post).not.toHaveBeenCalled()
  }
)

it('keeps expired status when the server refuses reactivation', async () => {
  const { put, post } = await renderKeysPage(3)
  put.mockResolvedValue({ data: { success: false, message: 'Token expired' } })
  await userEvent.click(screen.getByRole('button', { name: 'Enable' }))
  await screen.findByText('Token expired')
  expect(screen.getByText('Expired')).toBeInTheDocument()
  expect(screen.queryByText('Enabled')).not.toBeInTheDocument()
  expect(post).not.toHaveBeenCalled()
})

it.each([true, false])(
  'fetches a full key only on explicit copy and honors permission success=%s',
  async (success) => {
    const user = userEvent.setup()
    const { post } = await renderKeysPage()
    post.mockResolvedValue(
      success
        ? { data: { success: true, data: { key: 'fake-key-for-test-only' } } }
        : { data: { success: false, message: 'Verification required' } }
    )
    const copy = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue()
    await user.click(screen.getByRole('button', { name: 'Open menu' }))
    expect(post).not.toHaveBeenCalled()
    await user.click(screen.getByRole('menuitem', { name: 'Copy Key' }))
    await waitFor(() => expect(post).toHaveBeenCalledWith('/api/token/7/key'))
    if (success) {
      await waitFor(() =>
        expect(copy).toHaveBeenCalledWith('sk-fake-key-for-test-only')
      )
    } else {
      await screen.findByText('Verification required')
      expect(copy).not.toHaveBeenCalled()
    }
  }
)

it('keeps full mobile information without a group section heading', async () => {
  const matchMedia = window.matchMedia
  vi.spyOn(window, 'matchMedia').mockImplementation((query) => ({
    ...matchMedia(query),
    matches: query.includes('max-width'),
  }))
  i18n.addResourceBundle('zh', 'translation', zh.translation)
  await i18n.changeLanguage('zh')
  try {
    await renderKeysPage()
    expect(screen.queryByRole('table')).not.toBeInTheDocument()
    expect(screen.getByText(zh.translation['Created'])).toBeInTheDocument()
    expect(screen.getByText(zh.translation['Last Used'])).toBeInTheDocument()
    expect(screen.getByText(zh.translation['Expires'])).toBeInTheDocument()
    expect(
      screen.queryByText(zh.translation['Group'], { exact: true })
    ).not.toBeInTheDocument()
    expect(screen.getByText('default')).toBeInTheDocument()
    expect(screen.getByText('1x')).toBeInTheDocument()
    expect(screen.getByText(zh.translation['Models'])).toBeInTheDocument()
    expect(
      screen.getByText(zh.translation['IP Restriction'])
    ).toBeInTheDocument()
  } finally {
    await i18n.changeLanguage('en')
  }
})

it('opens complete model and IP restriction lists by tapping', async () => {
  const matchMedia = window.matchMedia
  vi.spyOn(window, 'matchMedia').mockImplementation((query) => ({
    ...matchMedia(query),
    matches: query.includes('max-width'),
  }))
  await renderKeysPage(1, {
    name: 'production-with-a-long-key-name',
    model_limits_enabled: true,
    model_limits: 'model-alpha,model-beta-with-a-long-name',
    allow_ips: '192.0.2.1\n2001:db8::1',
  })
  await userEvent.click(screen.getByRole('button', { name: /Models: 2 model/ }))
  let details = await screen.findByRole('dialog')
  expect(within(details).getByText('model-alpha')).toBeVisible()
  expect(within(details).getByText('model-beta-with-a-long-name')).toBeVisible()
  await userEvent.keyboard('{Escape}')
  await userEvent.click(
    screen.getByRole('button', { name: /IP Restriction: 2 IP/ })
  )
  details = await screen.findByRole('dialog')
  expect(within(details).getByText('192.0.2.1')).toBeVisible()
  expect(within(details).getByText('2001:db8::1')).toBeVisible()
})
