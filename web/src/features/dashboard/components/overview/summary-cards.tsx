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
import { Flame, Gauge, Zap } from 'lucide-react'
import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'

import { StaggerContainer, StaggerItem } from '@/components/page-transition'
import { getUserQuotaDates } from '@/features/dashboard/api'
import { safeDivide } from '@/features/dashboard/lib'
import { useSummaryCardsConfig } from '@/features/dashboard/hooks/use-dashboard-config'
import type { QuotaDataItem } from '@/features/dashboard/types'
import { useStatus } from '@/hooks/use-status'
import { formatNumber } from '@/lib/format'
import { requireServerSuccess } from '@/lib/server-error-message'
import { computeTimeRange } from '@/lib/time'
import { useAuthStore } from '@/stores/auth-store'

import { StatCard } from '../ui/stat-card'

const SUMMARY_SPARKLINE_BUCKETS = 12

// The sparkline window is the last 24 hours, so intensity averages use the
// matching minute count.
const MINUTES_PER_DAY = 1440

// Sparkline buckets are keyed by usage metric; card config keys
// ('recentTokens' / 'recentRequests') map onto these in getSummarySparkline.
type SummarySparklineKey = 'tokens' | 'requests'

function getBucketIndex(
  timestamp: number,
  start: number,
  end: number,
  bucketCount: number
): number {
  if (end <= start) return 0
  const ratio = (timestamp - start) / (end - start)
  return Math.min(bucketCount - 1, Math.max(0, Math.floor(ratio * bucketCount)))
}

function buildSummarySparklines(
  data: QuotaDataItem[],
  start: number,
  end: number
): Record<SummarySparklineKey, number[]> {
  const tokens = Array.from({ length: SUMMARY_SPARKLINE_BUCKETS }, () => 0)
  const requests = Array.from({ length: SUMMARY_SPARKLINE_BUCKETS }, () => 0)

  for (const item of data) {
    const timestamp = Number(item.created_at) || start
    const index = getBucketIndex(
      timestamp,
      start,
      end,
      SUMMARY_SPARKLINE_BUCKETS
    )
    tokens[index] += Number(item.token_used) || 0
    requests[index] += Number(item.count) || 0
  }

  return { tokens, requests }
}

function getSummarySparkline(
  key: string,
  sparklineData: Record<SummarySparklineKey, number[]>
): number[] | undefined {
  if (key === 'recentTokens') return sparklineData.tokens
  if (key === 'recentRequests') return sparklineData.requests
  return undefined
}

export function SummaryCards() {
  const { t } = useTranslation()
  const user = useAuthStore((state) => state.auth.user)
  const { loading } = useStatus()

  const summaryTimeRange = useMemo(() => computeTimeRange(1), [])
  const requestCount = Number(user?.request_count ?? 0)

  const usageTrendQuery = useQuery({
    queryKey: [
      'dashboard',
      'overview',
      'summary-sparklines',
      summaryTimeRange.start_timestamp,
      summaryTimeRange.end_timestamp,
    ],
    queryFn: async () =>
      requireServerSuccess(
        await getUserQuotaDates({
          start_timestamp: summaryTimeRange.start_timestamp,
          end_timestamp: summaryTimeRange.end_timestamp,
          default_time: 'hour',
        })
      ),
    staleTime: 60 * 1000,
  })

  const usageSummary = useMemo(() => {
    const rows = usageTrendQuery.data?.data ?? []
    let tokens = 0
    let requests = 0
    for (const row of rows) {
      tokens += Number(row.token_used) || 0
      requests += Number(row.count) || 0
    }
    const sparklines = buildSummarySparklines(
      rows,
      summaryTimeRange.start_timestamp,
      summaryTimeRange.end_timestamp
    )
    const peakTokens = sparklines.tokens.reduce(
      (highest, value) => Math.max(highest, value),
      0
    )
    return { tokens, requests, sparklines, peakTokens }
  }, [
    summaryTimeRange.end_timestamp,
    summaryTimeRange.start_timestamp,
    usageTrendQuery.data?.data,
  ])

  const items = useSummaryCardsConfig({
    recentTokensDisplay: formatNumber(usageSummary.tokens),
    recentRequestsDisplay: formatNumber(usageSummary.requests),
    requestCountDisplay: formatNumber(requestCount),
  }).map((config, index) => {
    const tones = ['accent-1', 'accent-2', 'accent-3'] as const

    return {
      key: config.key,
      title: config.title,
      value: config.value,
      desc: config.description,
      icon: config.icon,
      tone: tones[index] ?? 'accent-3',
      sparkline: getSummarySparkline(config.key, usageSummary.sparklines),
      sparklineVariant: 'line' as const,
    }
  })

  return (
    <div className='bg-card overflow-hidden rounded-2xl border shadow-xs'>
      <div className='grid xl:grid-cols-[minmax(0,1fr)_19rem]'>
        <div className='flex flex-col gap-2.5 p-3 sm:gap-3 sm:p-5'>
          <div className='flex flex-wrap items-start justify-between gap-3'>
            <div className='flex flex-col gap-1'>
              <h3 className='text-sm font-semibold sm:text-base'>
                {t('Usage at a glance')}
              </h3>
              <p className='text-muted-foreground text-xs sm:text-sm'>
                {t('Aggregated usage metrics and trend charts.')}
              </p>
            </div>
          </div>
          <StaggerContainer className='grid grid-cols-3 gap-1.5 sm:gap-3'>
            {items.map((it) => (
              <StaggerItem
                key={it.key}
                className='bg-background/60 rounded-lg border px-2 py-1.5 sm:rounded-xl sm:p-3'
              >
                <StatCard
                  title={it.title}
                  value={it.value}
                  description={it.desc}
                  icon={it.icon}
                  tone={it.tone}
                  sparkline={it.sparkline}
                  sparklineVariant={it.sparklineVariant}
                  loading={loading}
                  compactMobile
                />
              </StaggerItem>
            ))}
          </StaggerContainer>
        </div>

        <div className='flex flex-col justify-between gap-3 border-t bg-[linear-gradient(135deg,color-mix(in_oklch,var(--overview-accent-2)_12%,var(--background))_0%,color-mix(in_oklch,oklch(0.82_0.04_155)_8%,var(--background))_48%,color-mix(in_oklch,var(--overview-accent-1)_7%,var(--background))_100%)] p-3 sm:gap-4 sm:p-5 xl:border-t-0 xl:border-l'>
          <div className='flex flex-col gap-2 sm:gap-3'>
            <div className='flex items-center justify-between'>
              <span className='text-muted-foreground text-xs font-medium'>
                {t('Peak')}
              </span>
              <Flame className='size-3.5 shrink-0' aria-hidden='true' />
            </div>

            <div className='font-mono text-xl font-semibold tracking-tight sm:text-2xl'>
              {formatNumber(usageSummary.peakTokens)}
            </div>

            <div className='grid grid-cols-2 gap-2'>
              <div className='bg-background/60 rounded-lg px-2.5 py-2'>
                <div className='text-muted-foreground flex items-center gap-1 text-[11px] leading-none font-medium'>
                  <Zap className='size-3 shrink-0' aria-hidden='true' />
                  <span className='truncate'>{t('Average TPM')}</span>
                </div>
                <div className='text-foreground mt-1.5 truncate text-xs font-semibold tabular-nums'>
                  {formatNumber(safeDivide(usageSummary.tokens, MINUTES_PER_DAY))}
                </div>
              </div>
              <div className='bg-background/60 rounded-lg px-2.5 py-2'>
                <div className='text-muted-foreground flex items-center gap-1 text-[11px] leading-none font-medium'>
                  <Gauge className='size-3 shrink-0' aria-hidden='true' />
                  <span className='truncate'>{t('Average RPM')}</span>
                </div>
                <div className='text-foreground mt-1.5 truncate text-xs font-semibold tabular-nums'>
                  {formatNumber(
                    safeDivide(usageSummary.requests, MINUTES_PER_DAY)
                  )}
                </div>
              </div>
            </div>

            <div className='text-muted-foreground text-[11px]'>
              {t('Consumed in the last 24 hours')}
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
