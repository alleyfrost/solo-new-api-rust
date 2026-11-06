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
import { useChannels } from './channels-provider'
import { ChannelTestDialog } from './dialogs/channel-test-dialog'
import { CopyChannelDialog } from './dialogs/copy-channel-dialog'
import { FetchModelsDialog } from './dialogs/fetch-models-dialog'
import { ChannelMutateDrawer } from './drawers/channel-mutate-drawer'

export function ChannelsDialogs() {
  const { open, setOpen, currentRow } = useChannels()

  return (
    <>
      {/* Channel Create/Update Drawer */}
      <ChannelMutateDrawer
        open={open === 'create-channel' || open === 'update-channel'}
        onOpenChange={(v) => !v && setOpen(null)}
        currentRow={open === 'update-channel' ? currentRow : null}
      />

      {/* Test Channel Dialog */}
      <ChannelTestDialog
        open={open === 'test-channel'}
        onOpenChange={(v) => !v && setOpen(null)}
      />

      {/* Fetch Models Dialog */}
      <FetchModelsDialog
        open={open === 'fetch-models'}
        onOpenChange={(v) => !v && setOpen(null)}
      />

      {/* Copy Channel Dialog */}
      <CopyChannelDialog
        open={open === 'copy-channel'}
        onOpenChange={(v) => !v && setOpen(null)}
      />
    </>
  )
}
