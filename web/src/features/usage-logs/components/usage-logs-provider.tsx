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
/* eslint-disable react-refresh/only-export-components */
import { createContext, useContext, useState, type ReactNode } from 'react'

import { ROLE } from '@/lib/roles'
import { useAuthStore } from '@/stores/auth-store'

// Single-user deployment: the admin view is always in effect; kept as a type for column keys.

export type LogsViewAccess = 'self' | 'admin' | 'root'

export function resolveLogsViewAccess(role: number): LogsViewAccess {
  if (role < ROLE.ADMIN) return 'self'
  return role === ROLE.SUPER_ADMIN ? 'root' : 'admin'
}

interface UsageLogsContextValue {
  selectedUserId: number | null
  setSelectedUserId: (userId: number | null) => void
  userInfoDialogOpen: boolean
  setUserInfoDialogOpen: (open: boolean) => void
  sensitiveVisible: boolean
  setSensitiveVisible: (visible: boolean) => void
}

const UsageLogsContext = createContext<UsageLogsContextValue | undefined>(
  undefined
)

export function UsageLogsProvider({ children }: { children: ReactNode }) {
  const [selectedUserId, setSelectedUserId] = useState<number | null>(null)
  const [userInfoDialogOpen, setUserInfoDialogOpen] = useState(false)
  const [sensitiveVisible, setSensitiveVisible] = useState(true)

  return (
    <UsageLogsContext.Provider
      value={{
        selectedUserId,
        setSelectedUserId,
        userInfoDialogOpen,
        setUserInfoDialogOpen,
        sensitiveVisible,
        setSensitiveVisible,
      }}
    >
      {children}
    </UsageLogsContext.Provider>
  )
}

export function useUsageLogsContext() {
  const context = useContext(UsageLogsContext)
  if (!context) {
    throw new Error('useUsageLogsContext must be used within UsageLogsProvider')
  }
  return context
}

/**
 * Usage logs are single-tenant: admins always get the all-users view, and
 * the former "all / only mine" switch is gone. Data fetching and admin-only
 * UI key off `isAdminView`, derived from the role alone.
 */
export function useLogsViewScope() {
  const role = useAuthStore((state) => state.auth.user?.role ?? ROLE.GUEST)
  const canManageScope = role >= ROLE.ADMIN
  // 单用户部署：不再区分“全部 / 仅自己”，管理员视图始终生效。
  const viewAccess = resolveLogsViewAccess(role)
  const isAdminView = viewAccess !== 'self'
  const isRootView = viewAccess === 'root'

  return { canManageScope, isAdminView, isRootView, viewAccess }
}
