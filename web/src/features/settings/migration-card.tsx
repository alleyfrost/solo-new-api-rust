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
import { DatabaseZap } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { TitledCard } from '@/components/ui/titled-card'
import { handleServerError } from '@/lib/handle-server-error'

import { migrateFromNewApi } from './api'
import type { MigrationResult } from './types'

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`
}

export function MigrationCard() {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [backup, setBackup] = useState(true)
  const [running, setRunning] = useState(false)
  const [result, setResult] = useState<MigrationResult | null>(null)

  const handleMigrate = async () => {
    setRunning(true)
    try {
      const response = await migrateFromNewApi(backup)
      if (!response.success || !response.data) {
        throw new Error(response.message ?? t('Migration failed'))
      }
      setResult(response.data)
      toast.success(t('Migration completed'))
      setOpen(false)
    } catch (error) {
      handleServerError(error, t('Migration failed'))
    } finally {
      setRunning(false)
    }
  }

  return (
    <TitledCard
      title={t('Migrate from new-api')}
      description={t(
        'Remove historical retry and billing metadata, rebuild the dashboard aggregate table and compact the database.'
      )}
      icon={<DatabaseZap className='h-4 w-4' />}
      iconTone='destructive'
      disableHoverEffect
    >
      <div className='space-y-3'>
        <p className='text-muted-foreground text-xs'>
          {t(
            'This permanently rewrites historical log metadata. Request and response bodies are never stored. Back up first if you are unsure.'
          )}
        </p>
        <div className='flex flex-wrap items-center justify-between gap-2'>
          <p className='text-muted-foreground text-xs'>
            {result
              ? t('Cleaned {{rows}} logs, database {{before}} → {{after}}', {
                  rows: result.stripped_rows.toLocaleString(),
                  before: formatBytes(result.db_size_before),
                  after: formatBytes(result.db_size_after),
                })
              : t('Not migrated yet.')}
          </p>
          <Button
            type='button'
            variant='destructive'
            disabled={running}
            onClick={() => setOpen(true)}
          >
            {running ? t('Migrating…') : t('Migrate from new-api')}
          </Button>
        </div>
        {result?.backup_path ? (
          <p className='text-muted-foreground font-mono text-xs break-all'>
            {t('Backup')}: {result.backup_path}
          </p>
        ) : null}
      </div>

      <AlertDialog open={open} onOpenChange={setOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t('Migrate from new-api?')}</AlertDialogTitle>
            <AlertDialogDescription>
              {t(
                'This deletes historical retry audit and pricing fields from usage logs, rebuilds the dashboard aggregation table, then vacuums the database. The operation cannot be undone without the backup.'
              )}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <label className='flex items-center gap-2 text-sm'>
            <Checkbox
              checked={backup}
              onCheckedChange={(checked) => setBackup(checked === true)}
            />
            {t('Back up the original database first')}
          </label>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={running}>
              {t('Cancel')}
            </AlertDialogCancel>
            <AlertDialogAction
              disabled={running}
              onClick={(event) => {
                event.preventDefault()
                void handleMigrate()
              }}
            >
              {running ? t('Migrating…') : t('Confirm migration')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </TitledCard>
  )
}
