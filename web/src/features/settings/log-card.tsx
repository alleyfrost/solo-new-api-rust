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
import { ScrollText } from 'lucide-react'
import { useEffect, useState } from 'react'
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
import { Input } from '@/components/ui/input'
import { Separator } from '@/components/ui/separator'
import { Switch } from '@/components/ui/switch'
import { TitledCard } from '@/components/ui/titled-card'
import { formatTimestampForInput, parseTimestampFromInput } from '@/lib/format'
import { handleServerError } from '@/lib/handle-server-error'

import { cleanupLogsBefore } from './api'
import {
  DEFAULT_LOG_RETENTION_DAYS,
  MAX_RANGE_DAYS,
  MAX_RETENTION_DAYS,
  MIN_RANGE_DAYS,
  MIN_RETENTION_DAYS,
  OPTION_KEYS,
} from './types'
import { useOptions, useSaveOptions } from './use-settings'

const HOURS_IN_DAY = 24
const CLEANUP_ROUNDS = 100

type LogValues = {
  consume: boolean
  export: boolean
  rangeDays: string
  retentionDays: string
}

function daysAgoInput(days: number): string {
  return formatTimestampForInput(
    Math.floor(Date.now() / 1000) - days * HOURS_IN_DAY * 3600
  )
}

export function LogCard() {
  const { t } = useTranslation()
  const { values: loaded, isLoading } = useOptions()
  const save = useSaveOptions()

  const [values, setValues] = useState<LogValues>({
    consume: true,
    export: true,
    rangeDays: '',
    retentionDays: String(DEFAULT_LOG_RETENTION_DAYS),
  })
  const [cutoff, setCutoff] = useState(() => daysAgoInput(7))
  const [cleanupDone, setCleanupDone] = useState<number | null>(null)
  const [confirmOpen, setConfirmOpen] = useState(false)

  useEffect(() => {
    setValues({
      consume: loaded[OPTION_KEYS.logConsume] !== 'false',
      export: loaded[OPTION_KEYS.logExport] !== 'false',
      rangeDays: loaded[OPTION_KEYS.logRangeDays] ?? '',
      retentionDays:
        loaded[OPTION_KEYS.logRetentionDays] ??
        String(DEFAULT_LOG_RETENTION_DAYS),
    })
  }, [loaded])

  const rangeDays = Number(values.rangeDays)
  const retentionDays = Number(values.retentionDays)
  const changes: Array<[string, string | boolean | number]> = []
  if (values.consume !== (loaded[OPTION_KEYS.logConsume] !== 'false')) {
    changes.push([OPTION_KEYS.logConsume, values.consume])
  }
  if (values.export !== (loaded[OPTION_KEYS.logExport] !== 'false')) {
    changes.push([OPTION_KEYS.logExport, values.export])
  }
  if (rangeDays > 0 && String(rangeDays) !== loaded[OPTION_KEYS.logRangeDays]) {
    changes.push([OPTION_KEYS.logRangeDays, rangeDays])
  }
  if (
    String(retentionDays) !==
    (loaded[OPTION_KEYS.logRetentionDays] ?? String(DEFAULT_LOG_RETENTION_DAYS))
  ) {
    changes.push([OPTION_KEYS.logRetentionDays, retentionDays])
  }

  const handleSave = async () => {
    if (
      !Number.isInteger(rangeDays) ||
      rangeDays < MIN_RANGE_DAYS ||
      rangeDays > MAX_RANGE_DAYS
    ) {
      toast.error(
        t('Export range must be between {{min}} and {{max}} days', {
          min: MIN_RANGE_DAYS,
          max: MAX_RANGE_DAYS,
        })
      )
      return
    }
    if (
      !Number.isInteger(retentionDays) ||
      retentionDays < MIN_RETENTION_DAYS ||
      retentionDays > MAX_RETENTION_DAYS
    ) {
      toast.error(
        t('Retention must be between {{min}} and {{max}} days', {
          min: MIN_RETENTION_DAYS,
          max: MAX_RETENTION_DAYS,
        })
      )
      return
    }
    try {
      await save.mutateAsync(changes)
      toast.success(t('Settings saved'))
    } catch (error) {
      handleServerError(error, t('Failed to save settings'))
    }
  }

  const runCleanup = async () => {
    const before = parseTimestampFromInput(cutoff)
    if (before <= 0) {
      toast.error(t('Pick a valid cutoff time'))
      return
    }
    if (before >= Math.floor(Date.now() / 1000)) {
      toast.error(t('The cutoff time must be in the past'))
      return
    }
    try {
      let deleted = 0
      let stoppedEarly = false
      for (let round = 0; round < CLEANUP_ROUNDS; round += 1) {
        const response = await cleanupLogsBefore(before)
        if (!response.success) {
          throw new Error(response.message ?? t('Failed to clean logs'))
        }
        deleted += response.data?.deleted ?? 0
        if (!response.data?.has_more) break
        stoppedEarly = round === CLEANUP_ROUNDS - 1
      }
      setCleanupDone(deleted)
      toast.success(
        t('Cleaned {{count}} log records', { count: deleted.toLocaleString() })
      )
      if (stoppedEarly) {
        toast.message(t('More matching records remain. Run the cleanup again.'))
      }
    } catch (error) {
      handleServerError(error, t('Failed to clean logs'))
    }
  }

  if (isLoading) {
    return (
      <TitledCard
        title={t('Log Maintenance')}
        icon={<ScrollText className='h-4 w-4' />}
        iconTone='info'
        disableHoverEffect
      >
        <p className='text-muted-foreground text-sm'>{t('Loading…')}</p>
      </TitledCard>
    )
  }

  return (
    <TitledCard
      title={t('Log Maintenance')}
      description={t(
        'What gets recorded, what can be exported, and how history is pruned.'
      )}
      icon={<ScrollText className='h-4 w-4' />}
      iconTone='info'
      disableHoverEffect
    >
      <div className='space-y-4'>
        <div className='flex items-center justify-between gap-4'>
          <div className='space-y-0.5'>
            <p className='text-sm font-medium'>{t('Record usage logs')}</p>
            <p className='text-muted-foreground text-xs'>
              {t('Turn off to stop writing new usage records.')}
            </p>
          </div>
          <Switch
            checked={values.consume}
            disabled={save.isPending}
            onCheckedChange={(checked) =>
              setValues((prev) => ({ ...prev, consume: checked }))
            }
          />
        </div>

        <div className='flex items-center justify-between gap-4'>
          <div className='space-y-0.5'>
            <p className='text-sm font-medium'>{t('Allow log export')}</p>
            <p className='text-muted-foreground text-xs'>
              {t(
                'Controls the sign-in user export; the admin export stays available.'
              )}
            </p>
          </div>
          <Switch
            checked={values.export}
            disabled={save.isPending}
            onCheckedChange={(checked) =>
              setValues((prev) => ({ ...prev, export: checked }))
            }
          />
        </div>

        <label className='flex flex-col gap-2 sm:flex-row sm:items-end sm:justify-between'>
          <span className='space-y-0.5'>
            <span className='block text-sm font-medium'>
              {t('Maximum export range (days)')}
            </span>
            <span className='text-muted-foreground block text-xs'>
              {t('Use {{min}}–{{max}} days.', {
                min: MIN_RANGE_DAYS,
                max: MAX_RANGE_DAYS,
              })}
            </span>
          </span>
          <Input
            className='w-32'
            type='number'
            min={MIN_RANGE_DAYS}
            max={MAX_RANGE_DAYS}
            value={values.rangeDays}
            disabled={save.isPending}
            onChange={(event) =>
              setValues((prev) => ({ ...prev, rangeDays: event.target.value }))
            }
          />
        </label>

        <label className='flex flex-col gap-2 sm:flex-row sm:items-end sm:justify-between'>
          <span className='space-y-0.5'>
            <span className='block text-sm font-medium'>
              {t('Log retention (days)')}
            </span>
            <span className='text-muted-foreground block text-xs'>
              {t(
                'Logs older than this are removed automatically. 0 disables the cleanup.'
              )}
            </span>
          </span>
          <Input
            className='w-32'
            type='number'
            min={MIN_RETENTION_DAYS}
            max={MAX_RETENTION_DAYS}
            value={values.retentionDays}
            disabled={save.isPending}
            onChange={(event) =>
              setValues((prev) => ({
                ...prev,
                retentionDays: event.target.value,
              }))
            }
          />
        </label>

        <div className='flex justify-end'>
          <Button
            type='button'
            disabled={save.isPending || changes.length === 0}
            onClick={() => void handleSave()}
          >
            {t('Save')}
          </Button>
        </div>

        <Separator />

        <div className='space-y-3'>
          <div className='space-y-0.5'>
            <p className='text-sm font-medium'>{t('Clean history logs')}</p>
            <p className='text-muted-foreground text-xs'>
              {t(
                'Deletes usage records created before the chosen time, in batches.'
              )}
            </p>
          </div>

          <div className='flex flex-col gap-2 sm:flex-row sm:items-center'>
            <Input
              className='sm:max-w-64'
              type='datetime-local'
              value={cutoff}
              onChange={(event) => setCutoff(event.target.value)}
            />
            <div className='flex flex-wrap gap-2'>
              {[1, 7, 30].map((days) => (
                <Button
                  key={days}
                  type='button'
                  variant='outline'
                  size='sm'
                  onClick={() => setCutoff(daysAgoInput(days))}
                >
                  {days === 1
                    ? t('24 hours ago')
                    : t('{{count}} days ago', { count: days })}
                </Button>
              ))}
            </div>
          </div>

          <div className='flex items-center justify-between gap-3'>
            <p className='text-muted-foreground text-xs'>
              {cleanupDone === null
                ? t('Nothing cleaned yet in this session.')
                : t('Cleaned {{count}} log records', {
                    count: cleanupDone.toLocaleString(),
                  })}
            </p>
            <Button
              type='button'
              variant='destructive'
              onClick={() => setConfirmOpen(true)}
            >
              {t('Clean logs')}
            </Button>
            <AlertDialog
              open={confirmOpen}
              onOpenChange={(next) => setConfirmOpen(next)}
            >
              <AlertDialogContent>
                <AlertDialogHeader>
                  <AlertDialogTitle>
                    {t('Clean logs before this time?')}
                  </AlertDialogTitle>
                  <AlertDialogDescription>
                    {t('Deleted usage records cannot be recovered.')}
                  </AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter>
                  <AlertDialogCancel>{t('Cancel')}</AlertDialogCancel>
                  <AlertDialogAction
                    onClick={() => {
                      setConfirmOpen(false)
                      void runCleanup()
                    }}
                  >
                    {t('Clean logs')}
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
          </div>
        </div>
      </div>
    </TitledCard>
  )
}
