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
import { Loader2, KeyRound } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { PasswordInput } from '@/components/password-input'
import { Button } from '@/components/ui/button'
import { TitledCard } from '@/components/ui/titled-card'
import { handleServerError } from '@/lib/handle-server-error'
import { accountPasswordSchema } from '@/lib/password-policy'
import { createServerError } from '@/lib/server-error-message'

import { changePassword } from '../api'

type PasswordCardProps = {
  hasPassword: boolean
  username: string
  onSuccess: () => void
}

export function PasswordCard(props: PasswordCardProps) {
  const { t } = useTranslation()
  const [originalPassword, setOriginalPassword] = useState('')
  const [newPassword, setNewPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const [saving, setSaving] = useState(false)

  const reset = () => {
    setOriginalPassword('')
    setNewPassword('')
    setConfirmPassword('')
  }

  const handleSubmit = async () => {
    if (props.hasPassword && !originalPassword) {
      toast.error(t('Please enter your current password'))
      return
    }
    if (!accountPasswordSchema.safeParse(newPassword).success) {
      toast.error(t('Use 8–128 characters.'))
      return
    }
    if (newPassword !== confirmPassword) {
      toast.error(t('Passwords do not match'))
      return
    }
    if (props.hasPassword && newPassword === originalPassword) {
      toast.error(t('New password must be different from current password'))
      return
    }

    setSaving(true)
    try {
      const response = await changePassword({
        password: newPassword,
        ...(props.hasPassword ? { original_password: originalPassword } : {}),
      })
      if (!response.success) {
        throw createServerError(
          response,
          t(props.hasPassword ? 'Failed to change password' : 'Failed to set password')
        )
      }
      reset()
      props.onSuccess()
      toast.success(
        t(
          props.hasPassword
            ? 'Password changed successfully'
            : 'Password set successfully'
        )
      )
    } catch (error) {
      handleServerError(
        error,
        t(props.hasPassword ? 'Failed to change password' : 'Failed to set password')
      )
    } finally {
      setSaving(false)
    }
  }

  const title = t(props.hasPassword ? 'Change Password' : 'Set Password')

  return (
    <TitledCard
      title={title}
      description={t(
        'Changing your password signs out every other device; this session stays active.'
      )}
      icon={<KeyRound className='h-4 w-4' />}
      iconTone='warning'
      disableHoverEffect
    >
      <form
        className='space-y-4'
        onSubmit={(event) => {
          event.preventDefault()
          void handleSubmit()
        }}
      >
        {props.hasPassword && (
          <div className='space-y-1.5'>
            <label htmlFor='password-current' className='text-sm font-medium'>
              {t('Current Password')}
            </label>
            <PasswordInput
              id='password-current'
              value={originalPassword}
              autoComplete='current-password'
              disabled={saving}
              onChange={(event) => setOriginalPassword(event.target.value)}
            />
          </div>
        )}

        <div className='space-y-1.5'>
          <label htmlFor='password-new' className='text-sm font-medium'>
            {t('New Password')}
          </label>
          <PasswordInput
            id='password-new'
            value={newPassword}
            autoComplete='new-password'
            disabled={saving}
            onChange={(event) => setNewPassword(event.target.value)}
          />
          <p className='text-muted-foreground text-xs'>
            {t('Use 8–128 characters.')}
          </p>
        </div>

        <div className='space-y-1.5'>
          <label htmlFor='password-confirm' className='text-sm font-medium'>
            {t('Confirm New Password')}
          </label>
          <PasswordInput
            id='password-confirm'
            value={confirmPassword}
            autoComplete='new-password'
            disabled={saving}
            onChange={(event) => setConfirmPassword(event.target.value)}
          />
        </div>

        <div className='flex justify-end gap-2'>
          <Button
            type='button'
            variant='outline'
            disabled={saving}
            onClick={reset}
          >
            {t('Clear')}
          </Button>
          <Button type='submit' disabled={saving}>
            {saving && <Loader2 className='size-4 animate-spin' />}
            {title}
          </Button>
        </div>
      </form>
    </TitledCard>
  )
}
