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
import { useQueryClient } from '@tanstack/react-query'
import { getRouteApi } from '@tanstack/react-router'
import {
  Plus,
  MoreHorizontal,
  Settings2,
  Tags,
  ListChecks,
} from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ConfirmDialog } from '@/components/confirm-dialog'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuCheckboxItem,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Label } from '@/components/ui/label'
import { Separator } from '@/components/ui/separator'
import { Switch } from '@/components/ui/switch'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import {
  ADMIN_PERMISSION_ACTIONS,
  ADMIN_PERMISSION_RESOURCES,
  hasPermission,
} from '@/lib/admin-permissions'
import { useAuthStore } from '@/stores/auth-store'

import { handleFixAbilities } from '../lib'
import type { ChannelSortBy } from '../types'
import { ChannelSortControl } from './channel-priority-controls'
import { useChannels } from './channels-provider'

const route = getRouteApi('/_authenticated/channels/')

export function ChannelsPrimaryButtons() {
  const { t } = useTranslation()
  const navigate = route.useNavigate()
  const {
    setOpen,
    setCurrentRow,
    enableTagMode,
    setEnableTagMode,
    sorting,
    setSorting,
    batchMode,
    setBatchMode,
    setPriorityActionContainer,
  } = useChannels()
  const queryClient = useQueryClient()
  const [showConsistencyDialog, setShowConsistencyDialog] = useState(false)
  const [isRepairingConsistency, setIsRepairingConsistency] = useState(false)
  const currentUser = useAuthStore((s) => s.auth.user)
  const canEditSensitive = hasPermission(
    currentUser,
    ADMIN_PERMISSION_RESOURCES.CHANNEL,
    ADMIN_PERMISSION_ACTIONS.SENSITIVE_WRITE
  )

  const handleTagModeToggle = (checked: boolean) => {
    localStorage.setItem('enable-tag-mode', String(checked))
    setEnableTagMode(checked)
  }

  const handleBatchModeToggle = (checked: boolean) => {
    setBatchMode(checked)
  }

  return (
    <>
      <div className='flex flex-wrap items-center gap-2'>
        <div ref={setPriorityActionContainer} className='empty:hidden' />
        {/* Desktop: Toggle switches visible */}
        <div className='bg-muted/40 hidden h-8 shrink-0 items-center gap-2.5 rounded-lg border px-2.5 sm:flex'>
          <div className='text-muted-foreground has-data-checked:text-foreground flex items-center gap-2'>
            <Label
              htmlFor='channel-batch-mode'
              className='cursor-pointer gap-1.5 text-xs whitespace-nowrap'
            >
              <ListChecks className='size-3.5' aria-hidden />
              {t('Batch Operations')}
            </Label>
            <Switch
              id='channel-batch-mode'
              size='sm'
              checked={batchMode}
              onCheckedChange={handleBatchModeToggle}
            />
          </div>
          <Separator orientation='vertical' className='my-2' />
          <div className='text-muted-foreground has-data-checked:text-foreground flex items-center gap-2'>
            <Label
              htmlFor='tag-mode'
              className='cursor-pointer gap-1.5 text-xs whitespace-nowrap'
            >
              <Tags className='size-3.5' aria-hidden />
              {t('Tag Mode')}
            </Label>
            <Switch
              id='tag-mode'
              size='sm'
              checked={enableTagMode}
              onCheckedChange={handleTagModeToggle}
            />
          </div>
        </div>

        <ChannelSortControl
          field={(sorting[0]?.id ?? 'priority') as ChannelSortBy}
          desc={sorting[0]?.desc ?? true}
          onChange={(field, desc) => {
            setSorting([{ id: field, desc }])
            void navigate({
              search: (previous) => ({ ...previous, page: 1 }),
              replace: true,
            })
          }}
        />

        {/* Create Channel */}
        <Tooltip>
          <TooltipTrigger render={<span className='inline-flex' />}>
            <Button
              onClick={() => {
                if (!canEditSensitive) return
                setCurrentRow(null)
                setOpen('create-channel')
              }}
              disabled={!canEditSensitive}
            >
              <Plus className='h-4 w-4' />
              <span className='max-sm:hidden'>{t('Create Channel')}</span>
              <span className='sm:hidden'>{t('Create')}</span>
            </Button>
          </TooltipTrigger>
          {!canEditSensitive && (
            <TooltipContent>
              {t('No permission to perform this action')}
            </TooltipContent>
          )}
        </Tooltip>

        {/* More Actions */}
        <DropdownMenu>
          <DropdownMenuTrigger
            render={<Button variant='outline' size='icon' />}
            aria-label={t('More actions')}
          >
            <MoreHorizontal className='size-4' aria-hidden />
          </DropdownMenuTrigger>
          <DropdownMenuContent align='end' className='w-56'>
            {/* Mobile-only: toggle switches */}
            <DropdownMenuCheckboxItem
              className='sm:hidden'
              checked={batchMode}
              onCheckedChange={handleBatchModeToggle}
            >
              <ListChecks className='mr-2 h-4 w-4' />
              {t('Batch Operations')}
            </DropdownMenuCheckboxItem>

            <DropdownMenuCheckboxItem
              className='sm:hidden'
              checked={enableTagMode}
              onCheckedChange={handleTagModeToggle}
            >
              <Tags className='mr-2 h-4 w-4' />
              {t('Tag Mode')}
            </DropdownMenuCheckboxItem>

            <DropdownMenuSeparator className='sm:hidden' />

            <DropdownMenuItem
              onSelect={(e) => {
                e.preventDefault()
                setShowConsistencyDialog(true)
              }}
            >
              {t('Repair Channel Consistency')}
              <DropdownMenuShortcut>
                <Settings2 className='h-4 w-4' />
              </DropdownMenuShortcut>
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      <ConfirmDialog
        open={showConsistencyDialog}
        onOpenChange={setShowConsistencyDialog}
        title={t('Repair channel consistency?')}
        desc={t(
          'This will rebuild the channel routing index from every channel configuration, including supported models, groups, priorities, and weights. Routing may be briefly incomplete while the rebuild is running. Continue?'
        )}
        confirmText={t('Repair')}
        isLoading={isRepairingConsistency}
        handleConfirm={async () => {
          setIsRepairingConsistency(true)
          try {
            await handleFixAbilities(queryClient, (_result) => {
              // eslint-disable-next-line no-console
              console.log('Repair channel consistency result:', _result)
            })
            setShowConsistencyDialog(false)
          } finally {
            setIsRepairingConsistency(false)
          }
        }}
      />
    </>
  )
}
