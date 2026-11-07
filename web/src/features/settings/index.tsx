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
import { useTranslation } from 'react-i18next'

import { SectionPageLayout } from '@/components/layout'

import { ConcurrencyCard } from './concurrency-card'
import { LogCard } from './log-card'
import { SiteCard } from './site-card'

export function Settings() {
  const { t } = useTranslation()

  return (
    <SectionPageLayout>
      <SectionPageLayout.Title>{t('System Settings')}</SectionPageLayout.Title>
      <SectionPageLayout.Content>
        <div className='space-y-4'>
          <SiteCard />
          <ConcurrencyCard />
          <LogCard />
          <p className='text-muted-foreground text-xs'>
            {t(
              'Routing, retries and channel health follow the built-in policy and are configured per channel.'
            )}
          </p>
        </div>
      </SectionPageLayout.Content>
    </SectionPageLayout>
  )
}
