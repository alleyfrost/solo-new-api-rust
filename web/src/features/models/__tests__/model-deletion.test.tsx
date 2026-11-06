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
import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { AxiosError } from 'axios'
import { useState } from 'react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { api } from '@/lib/api'
import { useAuthStore } from '@/stores/auth-store'

import { ModelDeleteDialog } from '../components/dialogs/model-delete-dialog'

const models = [
  { id: 7, model_name: 'example-model', name_rule: 0 },
  { id: 8, model_name: 'another-model', name_rule: 0 },
]

function Fixture(props: { batch?: boolean; onSuccess?: () => void }) {
  const [open, setOpen] = useState(true)
  return (
    <>
      <button type='button' onClick={() => setOpen(true)}>
        Open deletion
      </button>
      {open && (
        <ModelDeleteDialog
          models={props.batch ? models : models.slice(0, 1)}
          onClose={() => setOpen(false)}
          onSuccess={props.onSuccess}
        />
      )}
    </>
  )
}

function mount(batch = false) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  const onSuccess = vi.fn()
  const invalidate = vi.spyOn(client, 'invalidateQueries')
  render(
    <QueryClientProvider client={client}>
      <Fixture batch={batch} onSuccess={onSuccess} />
    </QueryClientProvider>
  )
  return { onSuccess, invalidate }
}

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
  useAuthStore.getState().auth.reset()
})

describe('model deletion', () => {
  it('asks for a plain confirmation without channel or pricing options', () => {
    vi.spyOn(api, 'delete').mockResolvedValue({ data: { success: true } })
    mount()
    expect(screen.getByRole('button', { name: 'Delete' })).toBeEnabled()
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument()
  })

  it('deletes one model through DELETE /api/models/{id} and refreshes model data', async () => {
    const remove = vi.spyOn(api, 'delete').mockResolvedValue({
      data: { success: true, data: { deleted: 1, unbound: 0 } },
    })
    const { onSuccess, invalidate } = mount()
    const user = userEvent.setup()
    await user.click(screen.getByRole('button', { name: 'Delete' }))
    await waitFor(() => expect(onSuccess).toHaveBeenCalledOnce())
    expect(remove).toHaveBeenCalledWith('/api/models/7')
    expect(remove).toHaveBeenCalledTimes(1)
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['models'] })
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['vendors'] })
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
  })

  it('sends one request per model for a batch selection', async () => {
    const remove = vi.spyOn(api, 'delete').mockResolvedValue({
      data: { success: true, data: { deleted: 1, unbound: 0 } },
    })
    const { onSuccess } = mount(true)
    const user = userEvent.setup()
    await user.click(screen.getByRole('button', { name: 'Delete' }))
    await waitFor(() => expect(onSuccess).toHaveBeenCalledOnce())
    expect(remove).toHaveBeenCalledWith('/api/models/7')
    expect(remove).toHaveBeenCalledWith('/api/models/8')
    expect(remove).toHaveBeenCalledTimes(2)
  })

  it.each([
    {
      name: 'business rejection',
      response: { success: false, message: '模型不存在' },
    },
    { name: 'HTTP rejection', response: null },
  ])(
    'shows the server reason for a $name and keeps the dialog open for retry',
    async ({ response }) => {
      const remove = vi.spyOn(api, 'delete')
      if (response) {
        remove.mockResolvedValue({ data: response })
      } else {
        const error = new AxiosError('Request failed with status code 409')
        error.response = {
          data: { message: '模型不存在' },
          status: 409,
          statusText: 'Conflict',
          headers: {},
          config: { headers: {} },
        } as typeof error.response
        remove.mockRejectedValue(error)
      }
      const { onSuccess } = mount()
      const user = userEvent.setup()
      await user.click(screen.getByRole('button', { name: 'Delete' }))
      expect(await screen.findByRole('alert')).toHaveTextContent('模型不存在')
      expect(onSuccess).not.toHaveBeenCalled()
      expect(screen.getByRole('alertdialog')).toBeVisible()

      remove.mockResolvedValue({
        data: { success: true, data: { deleted: 1, unbound: 0 } },
      })
      await user.click(screen.getByRole('button', { name: 'Delete' }))
      await waitFor(() => expect(onSuccess).toHaveBeenCalledOnce())
      expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    }
  )

  it('disables dismissal and repeated submission while a removal is pending', async () => {
    let complete!: (response: unknown) => void
    const remove = vi.spyOn(api, 'delete').mockImplementation(
      () =>
        new Promise((resolve) => {
          complete = resolve
        })
    )
    const { onSuccess } = mount()
    const user = userEvent.setup()
    await user.click(screen.getByRole('button', { name: 'Delete' }))
    expect(screen.getByRole('button', { name: 'Delete' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeDisabled()
    await user.keyboard('{Escape}')
    expect(screen.getByRole('alertdialog')).toBeVisible()
    expect(remove).toHaveBeenCalledTimes(1)
    complete({ data: { success: true, data: { deleted: 1, unbound: 0 } } })
    await waitFor(() => expect(onSuccess).toHaveBeenCalledOnce())
  })
})
