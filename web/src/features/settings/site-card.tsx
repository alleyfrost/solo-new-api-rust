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
import { useQuery } from '@tanstack/react-query'
import type { TFunction } from 'i18next'
import { Building2 } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { MultiSelect } from '@/components/multi-select'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { TitledCard } from '@/components/ui/titled-card'
import { getUserModels } from '@/lib/api'
import { handleServerError } from '@/lib/handle-server-error'

import {
  ADDRESS_MAX_LENGTH,
  LOGO_MAX_LENGTH,
  MAX_IMAGE_MODELS,
  NOTICE_MAX_LENGTH,
  OPTION_KEYS,
  SYSTEM_NAME_MAX_LENGTH,
} from './types'
import { useOptions, useSaveOptions } from './use-settings'

type SiteValues = {
  systemName: string
  logo: string
  serverAddress: string
  notice: string
  imageModels: string
}

const EMPTY: SiteValues = {
  systemName: '',
  logo: '',
  serverAddress: '',
  notice: '',
  imageModels: '',
}

function splitModels(value: string): string[] {
  return value
    .split(',')
    .map((name) => name.trim())
    .filter(Boolean)
}

function validate(values: SiteValues, t: TFunction) {
  const name = values.systemName.trim()
  if (!name) return t('System name cannot be empty')
  if (name.length > SYSTEM_NAME_MAX_LENGTH) {
    return t('System name must be at most {{max}} characters', {
      max: SYSTEM_NAME_MAX_LENGTH,
    })
  }
  const logo = values.logo.trim()
  if (
    logo &&
    !['http://', 'https://', '/', 'data:image/'].some((prefix) =>
      logo.startsWith(prefix)
    )
  ) {
    return t('Logo must be an http(s) URL, a site path, or a data:image URI')
  }
  const address = values.serverAddress.trim()
  if (address && !/^https?:\/\/\S+$/.test(address)) {
    return t('Server address must be a full http(s) URL')
  }
  if (values.notice.length > NOTICE_MAX_LENGTH) {
    return t('Notice must be at most {{max}} characters', {
      max: NOTICE_MAX_LENGTH,
    })
  }
  const models = splitModels(values.imageModels)
  if (models.length > MAX_IMAGE_MODELS) {
    return t('At most {{max}} image models', { max: MAX_IMAGE_MODELS })
  }
  if (models.some((name) => !/^[A-Za-z0-9._:/@+-]{1,128}$/.test(name))) {
    return t(
      'Image model names may only contain letters, digits and - . _ : / @ +'
    )
  }
  return null
}

export function SiteCard() {
  const { t } = useTranslation()
  const { values: loaded, isLoading } = useOptions()
  const save = useSaveOptions()
  const [values, setValues] = useState<SiteValues>(EMPTY)

  const modelsQuery = useQuery({
    queryKey: ['settings', 'image-models', 'available'],
    queryFn: async () => {
      const response = await getUserModels()
      return response.success && Array.isArray(response.data)
        ? response.data
        : []
    },
    staleTime: 5 * 60 * 1000,
  })

  const imageModels = splitModels(values.imageModels)
  const imageModelOptions = useMemo(() => {
    const selected = splitModels(values.imageModels)
    const names = new Set([...(modelsQuery.data ?? []), ...selected])
    return [...names].map((model) => ({ label: model, value: model }))
  }, [modelsQuery.data, values.imageModels])

  useEffect(() => {
    setValues({
      systemName: loaded[OPTION_KEYS.systemName] ?? '',
      logo: loaded[OPTION_KEYS.logo] ?? '',
      serverAddress: loaded[OPTION_KEYS.serverAddress] ?? '',
      notice: loaded[OPTION_KEYS.notice] ?? '',
      imageModels: loaded[OPTION_KEYS.imageModels] ?? '',
    })
  }, [loaded])

  const changes: Array<[string, string]> = []
  if (values.systemName.trim() !== loaded[OPTION_KEYS.systemName]) {
    changes.push([OPTION_KEYS.systemName, values.systemName.trim()])
  }
  if (values.logo.trim() !== loaded[OPTION_KEYS.logo]) {
    changes.push([OPTION_KEYS.logo, values.logo.trim()])
  }
  if (values.serverAddress.trim() !== loaded[OPTION_KEYS.serverAddress]) {
    changes.push([OPTION_KEYS.serverAddress, values.serverAddress.trim()])
  }
  if (values.notice !== loaded[OPTION_KEYS.notice]) {
    changes.push([OPTION_KEYS.notice, values.notice])
  }
  const normalizedImageModels = splitModels(values.imageModels).join(',')
  if (normalizedImageModels !== loaded[OPTION_KEYS.imageModels]) {
    changes.push([OPTION_KEYS.imageModels, normalizedImageModels])
  }

  const handleSave = async () => {
    const problem = validate(values, t)
    if (problem) {
      toast.error(problem)
      return
    }
    try {
      await save.mutateAsync(changes)
      toast.success(t('Settings saved'))
    } catch (error) {
      handleServerError(error, t('Failed to save settings'))
    }
  }

  const field =
    (key: keyof SiteValues) => (event: { target: { value: string } }) =>
      setValues((prev) => ({ ...prev, [key]: event.target.value }))

  return (
    <TitledCard
      title={t('Site Information')}
      description={t(
        'Branding shown across the interface, plus the public API address.'
      )}
      icon={<Building2 className='h-4 w-4' />}
      iconTone='info'
      disableHoverEffect
    >
      {isLoading ? (
        <p className='text-muted-foreground text-sm'>{t('Loading…')}</p>
      ) : (
        <div className='space-y-4'>
          <div className='grid gap-4 sm:grid-cols-2'>
            <div className='space-y-1.5'>
              <label
                htmlFor='settings-system-name'
                className='text-sm font-medium'
              >
                {t('System Name')}
              </label>
              <Input
                id='settings-system-name'
                value={values.systemName}
                maxLength={SYSTEM_NAME_MAX_LENGTH}
                onChange={field('systemName')}
                disabled={save.isPending}
              />
              <p className='text-muted-foreground text-xs'>
                {t('Use 1–{{max}} characters.', {
                  max: SYSTEM_NAME_MAX_LENGTH,
                })}
              </p>
            </div>
            <div className='space-y-1.5'>
              <label htmlFor='settings-logo' className='text-sm font-medium'>
                {t('Logo URL')}
              </label>
              <Input
                id='settings-logo'
                value={values.logo}
                maxLength={LOGO_MAX_LENGTH}
                placeholder={t('Leave empty to use the built-in logo')}
                onChange={field('logo')}
                disabled={save.isPending}
              />
            </div>
          </div>

          <div className='space-y-1.5'>
            <label
              htmlFor='settings-server-address'
              className='text-sm font-medium'
            >
              {t('Server Address')}
            </label>
            <Input
              id='settings-server-address'
              value={values.serverAddress}
              maxLength={ADDRESS_MAX_LENGTH}
              placeholder={window.location.origin}
              onChange={field('serverAddress')}
              disabled={save.isPending}
            />
            <p className='text-muted-foreground text-xs'>
              {t(
                'Used when copying client configurations. Empty means the current domain.'
              )}
            </p>
          </div>

          <div className='space-y-1.5'>
            <label htmlFor='settings-notice' className='text-sm font-medium'>
              {t('Notice')}
            </label>
            <Textarea
              id='settings-notice'
              value={values.notice}
              maxLength={NOTICE_MAX_LENGTH}
              rows={3}
              onChange={(event) =>
                setValues((prev) => ({ ...prev, notice: event.target.value }))
              }
              disabled={save.isPending}
            />
            <p className='text-muted-foreground text-xs'>
              {t('Shown on the sign-in page.')}
            </p>
          </div>

          <div className='space-y-1.5'>
            <label
              htmlFor='settings-image-models'
              className='text-sm font-medium'
            >
              {t('Image Models')}
            </label>
            <MultiSelect
              id='settings-image-models'
              options={imageModelOptions}
              selected={imageModels}
              onChange={(models) =>
                setValues((prev) => ({
                  ...prev,
                  imageModels: models.join(','),
                }))
              }
              placeholder={t('Select models or add custom ones')}
              allowCreate
              createLabel='Add custom model "{{value}}"'
              maxVisibleChips={8}
              disabled={save.isPending}
            />
            <p className='text-muted-foreground text-xs'>
              {t(
                'Only image-generation models. Chat requests to them are turned into image generation.'
              )}
            </p>
          </div>

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
