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
import { Gauge } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { TitledCard } from '@/components/ui/titled-card'
import { handleServerError } from '@/lib/handle-server-error'

import {
  DEFAULT_BODY_GATE_BIG,
  DEFAULT_BODY_GATE_HUGE,
  MAX_BODY_GATE,
  MIN_BODY_GATE,
  OPTION_KEYS,
} from './types'
import { useOptions, useSaveOptions } from './use-settings'

export function ConcurrencyCard() {
  const { t } = useTranslation()
  const { values: loaded, isLoading } = useOptions()
  const save = useSaveOptions()

  const [big, setBig] = useState(String(DEFAULT_BODY_GATE_BIG))
  const [huge, setHuge] = useState(String(DEFAULT_BODY_GATE_HUGE))

  useEffect(() => {
    setBig(loaded[OPTION_KEYS.bodyGateBig] ?? String(DEFAULT_BODY_GATE_BIG))
    setHuge(loaded[OPTION_KEYS.bodyGateHuge] ?? String(DEFAULT_BODY_GATE_HUGE))
  }, [loaded])

  const changes: Array<[string, string | number]> = []
  if (
    big !== (loaded[OPTION_KEYS.bodyGateBig] ?? String(DEFAULT_BODY_GATE_BIG))
  ) {
    changes.push([OPTION_KEYS.bodyGateBig, Number(big)])
  }
  if (
    huge !==
    (loaded[OPTION_KEYS.bodyGateHuge] ?? String(DEFAULT_BODY_GATE_HUGE))
  ) {
    changes.push([OPTION_KEYS.bodyGateHuge, Number(huge)])
  }

  const handleSave = async () => {
    const bigValue = Number(big)
    const hugeValue = Number(huge)
    if (
      !Number.isInteger(bigValue) ||
      !Number.isInteger(hugeValue) ||
      bigValue < MIN_BODY_GATE ||
      bigValue > MAX_BODY_GATE ||
      hugeValue < MIN_BODY_GATE ||
      hugeValue > MAX_BODY_GATE
    ) {
      toast.error(
        t('Concurrency must be an integer between {{min}} and {{max}}', {
          min: MIN_BODY_GATE,
          max: MAX_BODY_GATE,
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

  return (
    <TitledCard
      title={t('Relay Concurrency')}
      description={t(
        'Limits how many large request bodies are parsed and forwarded at the same time. Higher values use more memory.'
      )}
      icon={<Gauge className='h-4 w-4' />}
      iconTone='warning'
      disableHoverEffect
    >
      {isLoading ? (
        <p className='text-muted-foreground text-sm'>{t('Loading…')}</p>
      ) : (
        <div className='space-y-4'>
          <div className='grid gap-4 sm:grid-cols-2'>
            <div className='space-y-1.5'>
              <label
                htmlFor='settings-body-gate-big'
                className='text-sm font-medium'
              >
                {t('2–8 MiB concurrency')}
              </label>
              <Input
                id='settings-body-gate-big'
                type='number'
                min={MIN_BODY_GATE}
                max={MAX_BODY_GATE}
                value={big}
                onChange={(event) => setBig(event.target.value)}
                disabled={save.isPending}
              />
            </div>
            <div className='space-y-1.5'>
              <label
                htmlFor='settings-body-gate-huge'
                className='text-sm font-medium'
              >
                {t('> 8 MiB concurrency')}
              </label>
              <Input
                id='settings-body-gate-huge'
                type='number'
                min={MIN_BODY_GATE}
                max={MAX_BODY_GATE}
                value={huge}
                onChange={(event) => setHuge(event.target.value)}
                disabled={save.isPending}
              />
            </div>
          </div>
          <p className='text-muted-foreground text-xs'>
            {t(
              'Requests above the limit wait in the queue before their body is read. Keep the values low if the container has limited memory.'
            )}
          </p>
          <div className='flex justify-end'>
            <Button
              type='button'
              disabled={save.isPending || changes.length === 0}
              onClick={() => void handleSave()}
            >
              {t('Save')}
            </Button>
          </div>
        </div>
      )}
    </TitledCard>
  )
}
