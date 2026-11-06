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
import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'

import { handleServerError } from '@/lib/handle-server-error'
import { createServerError } from '@/lib/server-error-message'

import { getChannelKey } from '../api'

export function useChannelKeyDisclosure(
  open: boolean,
  channelId: number | null
) {
  const { t } = useTranslation()
  const [disclosedKey, setDisclosedKey] = useState<{
    channelId: number
    key: string
  } | null>(null)
  const [isChannelKeyLoading, setIsChannelKeyLoading] = useState(false)
  const operation = useRef<AbortController | null>(null)

  useEffect(() => {
    setDisclosedKey(null)
    setIsChannelKeyLoading(false)
    return () => {
      operation.current?.abort()
      operation.current = null
    }
  }, [open, channelId])

  const handleRevealKey = useCallback(async () => {
    if (!channelId || !open || operation.current) return
    const current = new AbortController()
    operation.current = current
    try {
      setIsChannelKeyLoading(true)
      const res = await getChannelKey(channelId, '', current.signal)
      if (operation.current !== current) return
      if (!res.success) {
        throw createServerError(res, t('Failed to fetch channel key'))
      }
      setDisclosedKey({ channelId, key: res.data?.key ?? '' })
      toast.success(t('Channel key unlocked'))
    } catch (error) {
      if (operation.current === current && !current.signal.aborted) {
        handleServerError(error)
      }
    } finally {
      if (operation.current === current) {
        operation.current = null
        setIsChannelKeyLoading(false)
      }
    }
  }, [channelId, open, t])

  const channelKey =
    open && disclosedKey?.channelId === channelId ? disclosedKey.key : null
  return { channelKey, isChannelKeyLoading, handleRevealKey }
}
