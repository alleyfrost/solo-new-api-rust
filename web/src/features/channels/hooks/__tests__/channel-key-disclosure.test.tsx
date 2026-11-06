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
import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'

import { api } from '@/lib/api'

import { useChannelKeyDisclosure } from '../use-channel-key-disclosure'

function Harness(props: { open: boolean; channelId: number }) {
  const disclosure = useChannelKeyDisclosure(props.open, props.channelId)
  return (
    <>
      <button type='button' onClick={disclosure.handleRevealKey}>
        Reveal
      </button>
      <output aria-label='Channel key'>
        {disclosure.channelKey ?? 'Hidden'}
      </output>
    </>
  )
}

function deferredResponse<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((finish) => {
    resolve = finish
  })
  return { promise, resolve }
}

afterEach(() => vi.restoreAllMocks())

it('reveals the channel key without extra verification', async () => {
  const post = vi.spyOn(api, 'post').mockResolvedValue({
    data: { success: true, data: { key: 'CHANNEL_SECRET' } },
  })
  const user = userEvent.setup()
  render(<Harness open channelId={123} />)
  await user.click(screen.getByRole('button', { name: 'Reveal' }))
  await waitFor(() =>
    expect(screen.getByLabelText('Channel key')).toHaveTextContent(
      'CHANNEL_SECRET'
    )
  )
  expect(post).toHaveBeenCalledWith(
    '/api/channel/123/key',
    undefined,
    expect.anything()
  )
})

it.each(['switch', 'close'] as const)(
  'discards a pending channel key response after %s',
  async (change) => {
    const keyReply = deferredResponse<{
      data: { success: boolean; data: { key: string } }
    }>()
    vi.spyOn(api, 'post').mockReturnValue(keyReply.promise)
    const user = userEvent.setup()
    const view = render(<Harness open channelId={123} />)
    await user.click(screen.getByRole('button', { name: 'Reveal' }))
    view.rerender(
      <Harness
        open={change !== 'close'}
        channelId={change === 'switch' ? 456 : 123}
      />
    )
    await act(async () => {
      keyReply.resolve({
        data: { success: true, data: { key: 'CHANNEL_A_SECRET' } },
      })
      await keyReply.promise
    })
    expect(screen.getByLabelText('Channel key')).toHaveTextContent('Hidden')
    expect(screen.queryByText('CHANNEL_A_SECRET')).not.toBeInTheDocument()
  }
)
