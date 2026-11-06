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
import type { Row } from '@tanstack/react-table'
import {
  render,
  screen,
  waitFor,
  cleanup,
  act,
  fireEvent,
} from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { AxiosError } from 'axios'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { DataTableRowActions } from '../components/data-table-row-actions'
import { ModelMutateDrawer } from '../components/drawers/model-mutate-drawer'
import { ModelsDialogs } from '../components/models-dialogs'
import { ModelsProvider } from '../components/models-provider'
import type { Model } from '../types'

const model = {
  id: 7,
  model_name: 'example-model',
  description: 'Original',
  status: 1,
  sync_official: 1,
  name_rule: 0,
  vendor_id: 3,
  endpoints: '',
  supported_endpoints: ['openai'],
  created_time: 1,
  updated_time: 1,
}

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
  useAuthStore.getState().auth.reset()
})

function renderModelActions(currentModel: Model = model, role = 100) {
  useAuthStore.getState().auth.setUser({ id: 1, username: 'admin', role })
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  render(
    <QueryClientProvider client={client}>
      <ModelsProvider>
        <DataTableRowActions row={{ original: currentModel } as Row<Model>} />
        <ModelsDialogs />
      </ModelsProvider>
    </QueryClientProvider>
  )
  return client
}

function renderDrawer(
  currentRow?: Model,
  onClose = vi.fn(),
  role = 100
): QueryClient {
  useAuthStore.getState().auth.setUser({ id: 1, username: 'root', role })
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  render(
    <QueryClientProvider client={client}>
      <ModelsProvider>
        <ModelMutateDrawer open onOpenChange={onClose} currentRow={currentRow} />
      </ModelsProvider>
    </QueryClientProvider>
  )
  return client
}

function mockReferenceData(
  handlers: (url: string) => unknown | undefined,
  vendorItems: Array<Record<string, unknown>> = []
) {
  return vi.spyOn(api, 'get').mockImplementation(async (url) => {
    const custom = handlers(url as string)
    if (custom) return { data: custom }
    if (url === '/api/vendors/') {
      return {
        data: {
          success: true,
          data: { items: vendorItems, total: vendorItems.length },
        },
      }
    }
    return { data: { success: true, data: { items: [] } } }
  })
}

describe('row actions', () => {
  it('offers edit, visibility toggle and delete without a pricing entry', async () => {
    mockReferenceData(() => undefined)
    renderModelActions()
    const user = userEvent.setup()
    expect(screen.getByRole('button', { name: 'Edit' })).toBeVisible()
    expect(
      screen.queryByRole('button', { name: 'Pricing' })
    ).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Open menu' }))
    expect(
      await screen.findByRole('menuitem', { name: 'Hide from model square' })
    ).toBeVisible()
    expect(screen.getByRole('menuitem', { name: 'Delete' })).toBeVisible()
  })

  it('offers only metadata creation for a channel model', () => {
    mockReferenceData(() => undefined)
    renderModelActions({ ...model, id: 0, model_name: 'channel-only' })
    expect(screen.getByRole('button', { name: 'Add metadata' })).toBeVisible()
    expect(
      screen.queryByRole('button', { name: 'Open menu' })
    ).not.toBeInTheDocument()
  })
})

describe('metadata editing', () => {
  it('limits the editor to the metadata and connections tabs', async () => {
    mockReferenceData((url) =>
      url === '/api/models/7'
        ? { success: true, data: model }
        : undefined
    )
    renderDrawer(model)
    await screen.findByLabelText('Description')
    const user = userEvent.setup()
    const tabs = screen.getAllByRole('tab')
    expect(tabs.map((tab) => tab.textContent)).toEqual([
      'Model metadata',
      'Channels and groups',
    ])
    await user.click(screen.getByRole('tab', { name: 'Channels and groups' }))
    expect(
      screen.queryByRole('textbox', { name: 'Model Name *' })
    ).not.toBeInTheDocument()
  })

  it.each([
    {
      name: 'business rejection',
      response: { success: false, message: '模型名称已存在' },
    },
    { name: 'HTTP rejection', response: null },
  ])(
    'shows the server reason for a $name and preserves the draft for retry',
    async ({ response }) => {
      useAuthStore
        .getState()
        .auth.setUser({ id: 2, username: 'admin', role: 10 })
      mockReferenceData(() => undefined)
      const post = vi.spyOn(api, 'post')
      if (response) {
        post.mockResolvedValueOnce({ data: response })
      } else {
        const error = new AxiosError('Request failed with status code 409')
        error.response = {
          data: { message: '模型名称已存在' },
          status: 409,
          statusText: 'Conflict',
          headers: {},
          config: { headers: {} },
        } as typeof error.response
        post.mockRejectedValueOnce(error)
      }
      post.mockResolvedValue({ data: { success: true } })
      const close = vi.fn()
      const fallbackError = vi.fn()
      const client = new QueryClient({
        defaultOptions: {
          queries: { retry: false },
          mutations: { retry: false, onError: fallbackError },
        },
      })
      render(
        <QueryClientProvider client={client}>
          <ModelMutateDrawer open onOpenChange={close} />
        </QueryClientProvider>
      )
      const user = userEvent.setup()
      await user.type(screen.getByLabelText('Model Name *'), 'duplicate-model')
      await user.type(screen.getByLabelText('Description'), 'Keep this draft')
      await user.click(screen.getByRole('button', { name: 'Save metadata' }))
      expect(await screen.findByRole('alert')).toHaveTextContent(
        '模型名称已存在'
      )
      expect(fallbackError).not.toHaveBeenCalled()
      expect(close).not.toHaveBeenCalled()
      expect(screen.getByLabelText('Model Name *')).toHaveValue(
        'duplicate-model'
      )
      expect(screen.getByLabelText('Description')).toHaveValue(
        'Keep this draft'
      )
      await user.clear(screen.getByLabelText('Model Name *'))
      await user.type(screen.getByLabelText('Model Name *'), 'unique-model')
      await user.click(screen.getByRole('button', { name: 'Save metadata' }))
      await waitFor(() => expect(close).toHaveBeenCalledWith(false))
      expect(screen.queryByRole('alert')).not.toBeInTheDocument()
      expect(post).toHaveBeenLastCalledWith(
        '/api/models/',
        expect.objectContaining({
          model_name: 'unique-model',
          description: 'Keep this draft',
        }),
        { skipBusinessError: true, skipErrorHandler: true }
      )
      client.clear()
    }
  )

  it.each([undefined, 0])(
    'does not copy a vendor to the next model when its vendor_id is %s',
    async (vendorId) => {
      const second = {
        ...model,
        id: 8,
        model_name: 'unassigned-model',
        vendor_id: vendorId,
      }
      let resolveDetail!: (value: Awaited<ReturnType<typeof api.get>>) => void
      const detail = new Promise<Awaited<ReturnType<typeof api.get>>>(
        (resolve) => {
          resolveDetail = resolve
        }
      )
      const get = vi.spyOn(api, 'get').mockImplementation(async (url) => {
        if (url === '/api/models/7') {
          return { data: { success: true, data: model } }
        }
        if (url === '/api/models/8') return detail
        if (url === '/api/vendors/') {
          return {
            data: {
              success: true,
              data: {
                items: [{ id: 3, name: 'Existing vendor' }],
              },
            },
          }
        }
        throw new Error(`Unexpected GET ${url}`)
      })
      const put = vi
        .spyOn(api, 'put')
        .mockResolvedValue({ data: { success: true } })
      const client = new QueryClient({
        defaultOptions: {
          queries: { retry: false },
          mutations: { retry: false },
        },
      })
      const close = vi.fn()
      const tree = (currentRow: Model, open: boolean) => (
        <QueryClientProvider client={client}>
          <ModelMutateDrawer
            open={open}
            currentRow={currentRow}
            onOpenChange={close}
          />
        </QueryClientProvider>
      )
      const view = render(tree(model, true))
      await waitFor(() =>
        expect(screen.getByRole('combobox', { name: 'Vendor' })).toHaveValue(
          'Existing vendor'
        )
      )
      fireEvent.click(screen.getAllByRole('button', { name: 'Close' })[0])
      expect(close).toHaveBeenCalledWith(false)
      view.rerender(tree(model, false))
      await waitFor(() =>
        expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
      )
      view.rerender(tree(second, true))
      await waitFor(() => expect(get).toHaveBeenCalledWith('/api/models/8'))
      expect(
        screen.getByRole('button', { name: 'Save metadata' })
      ).toBeDisabled()
      await act(async () =>
        resolveDetail({ data: { success: true, data: second } })
      )
      await waitFor(() =>
        expect(screen.getByLabelText('Model Name *')).toHaveValue(
          'unassigned-model'
        )
      )
      expect(screen.getByRole('combobox', { name: 'Vendor' })).toHaveValue('')
      fireEvent.click(screen.getByRole('button', { name: 'Save metadata' }))
      await waitFor(() =>
        expect(put).toHaveBeenCalledWith(
          '/api/models/',
          expect.objectContaining({ id: 8, vendor_id: 0 }),
          { skipBusinessError: true, skipErrorHandler: true }
        )
      )
      view.unmount()
      client.clear()
    }
  )

  it('saves every writable metadata field through /api/models only', async () => {
    useAuthStore.getState().auth.setUser({ id: 2, username: 'admin', role: 10 })
    const get = mockReferenceData(
      (url) =>
        url === '/api/models/7' ? { success: true, data: model } : undefined,
      [
        { id: 3, name: 'Existing vendor', icon: 'Gemini.Color' },
        { id: 4, name: 'Another vendor', icon: 'Gemini.Color' },
      ]
    )
    const put = vi
      .spyOn(api, 'put')
      .mockResolvedValue({ data: { success: true, data: model } })
    renderDrawer(model)
    const description = await screen.findByLabelText('Description')
    await waitFor(() => expect(description).toHaveValue('Original'))
    expect(screen.getByRole('combobox', { name: 'Vendor' })).toHaveValue(
      'Existing vendor'
    )
    const user = userEvent.setup()
    expect(screen.getByText('Gemini.Color')).toBeVisible()
    await user.click(screen.getByRole('button', { name: 'Custom model icon' }))
    const icon = screen.getByRole('combobox', { name: 'Icon' })
    await user.type(icon, 'Claude.Avatar')
    await user.keyboard('{Escape}')
    expect(screen.getByText('Claude.Avatar')).toBeVisible()
    await user.click(
      screen.getByRole('button', { name: 'Inherit vendor icon' })
    )
    expect(
      screen.queryByRole('combobox', { name: 'Icon' })
    ).not.toBeInTheDocument()
    expect(screen.getByText('Gemini.Color')).toBeVisible()
    const vendorInput = screen.getByRole('combobox', { name: 'Vendor' })
    await user.click(vendorInput)
    await user.type(vendorInput, 'Another')
    await user.click(screen.getByRole('option', { name: 'Another vendor' }))
    expect(vendorInput).toHaveValue('Another vendor')
    await user.clear(description)
    await user.type(description, 'Updated metadata')
    await user.click(
      screen.getByRole('button', { name: /Update Model|Save metadata/ })
    )
    await waitFor(() => expect(put).toHaveBeenCalled())
    expect(put.mock.calls.every(([url]) => url === '/api/models/')).toBe(true)
    expect(get.mock.calls.every(([url]) => url !== '/api/models/')).toBe(true)
    expect(put.mock.calls[0][1]).toMatchObject({
      description: 'Updated metadata',
      icon: '',
      model_name: 'example-model',
      vendor_id: 4,
      endpoints: '',
      status: 1,
      sync_official: 1,
    })
  })

  it('writes the display policy and sync policy switches back to the server', async () => {
    mockReferenceData((url) =>
      url === '/api/models/7' ? { success: true, data: model } : undefined
    )
    const put = vi
      .spyOn(api, 'put')
      .mockResolvedValue({ data: { success: true, data: model } })
    renderDrawer(model)
    await screen.findByLabelText('Description')
    const user = userEvent.setup()
    expect(
      screen.getByRole('switch', { name: 'Model square visibility' })
    ).toBeChecked()
    expect(
      screen.getByRole('switch', { name: 'Allow metadata sync' })
    ).toBeChecked()
    await user.click(
      screen.getByRole('switch', { name: 'Model square visibility' })
    )
    await user.click(screen.getByRole('switch', { name: 'Allow metadata sync' }))
    await user.click(screen.getByRole('button', { name: 'Save metadata' }))
    await waitFor(() => expect(put).toHaveBeenCalled())
    expect(put.mock.calls[0][1]).toMatchObject({
      id: 7,
      status: 0,
      sync_official: 0,
    })
  })
})
