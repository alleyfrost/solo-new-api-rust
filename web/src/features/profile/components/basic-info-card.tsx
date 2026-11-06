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
import { IdCard } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { TitledCard } from '@/components/ui/titled-card'
import { handleServerError } from '@/lib/handle-server-error'
import { createServerError } from '@/lib/server-error-message'

import { updateUserProfile } from '../api'
import type { UserProfile } from '../types'

const DISPLAY_NAME_MAX_LENGTH = 20

type BasicInfoCardProps = {
  profile: UserProfile | null
  onProfileUpdate: () => void
}

export function BasicInfoCard(props: BasicInfoCardProps) {
  const { t } = useTranslation()
  const [displayName, setDisplayName] = useState('')
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    setDisplayName(props.profile?.display_name ?? '')
  }, [props.profile?.display_name])

  const trimmed = displayName.trim()
  const unchanged = trimmed === (props.profile?.display_name ?? '')

  const handleSave = async () => {
    if (!trimmed) {
      toast.error(t('Display name cannot be empty'))
      return
    }
    if (trimmed.length > DISPLAY_NAME_MAX_LENGTH) {
      toast.error(t('Display name must be at most {{max}} characters', { max: DISPLAY_NAME_MAX_LENGTH }))
      return
    }
    setSaving(true)
    try {
      const response = await updateUserProfile({ display_name: trimmed })
      if (!response.success) {
        throw createServerError(response, t('Failed to update profile'))
      }
      props.onProfileUpdate()
      toast.success(t('Profile updated successfully'))
    } catch (error) {
      handleServerError(error, t('Failed to update profile'))
    } finally {
      setSaving(false)
    }
  }

  return (
    <TitledCard
      title={t('Basic Information')}
      description={t('How your account appears across the interface.')}
      icon={<IdCard className='h-4 w-4' />}
      iconTone='info'
      disableHoverEffect
    >
      <div className='space-y-4'>
        <div className='flex flex-col gap-2 sm:flex-row sm:items-end sm:gap-3'>
          <div className='flex-1 space-y-1.5'>
            <label
              htmlFor='profile-display-name'
              className='text-sm font-medium'
            >
              {t('Display Name')}
            </label>
            <Input
              id='profile-display-name'
              value={displayName}
              maxLength={DISPLAY_NAME_MAX_LENGTH}
              disabled={saving}
              onChange={(event) => setDisplayName(event.target.value)}
              placeholder={props.profile?.username}
            />
            <p className='text-muted-foreground text-xs'>
              {t('Use 1–{{max}} characters.', { max: DISPLAY_NAME_MAX_LENGTH })}
            </p>
          </div>
          <Button
            type='button'
            disabled={saving || unchanged}
            onClick={() => void handleSave()}
          >
            {t('Save')}
          </Button>
        </div>

        <dl className='text-muted-foreground grid grid-cols-1 gap-2 text-xs sm:grid-cols-2 sm:text-sm'>
          <div className='flex gap-2'>
            <dt className='shrink-0'>{t('Username')}</dt>
            <dd className='truncate'>@{props.profile?.username}</dd>
          </div>
          <div className='flex gap-2'>
            <dt className='shrink-0'>{t('Group')}</dt>
            <dd className='truncate'>{props.profile?.group || 'default'}</dd>
          </div>
        </dl>
      </div>
    </TitledCard>
  )
}
