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
const legacyOrigin = 'https://legacy-route.invalid'

const legacyConsoleRoutes: Record<string, string> = {
  '/console': '/dashboard',
  '/console/models': '/models',
  '/console/deployment': '/models/metadata',
  '/console/channel': '/channels',
  '/console/token': '/keys',
  '/console/playground': '/playground',
  '/console/personal': '/profile',
  '/console/log': '/usage-logs',
}

function normalizeLegacyPath(pathname: string): string {
  if (pathname === '/') return pathname
  return pathname.replace(/\/+$/, '')
}

function buildTargetHref(targetPath: string, source: URL): string {
  const target = new URL(targetPath, legacyOrigin)
  source.searchParams.forEach((value, key) => {
    target.searchParams.append(key, value)
  })
  target.hash = source.hash
  return `${target.pathname}${target.search}${target.hash}`
}

export function resolveLegacyRoute(rawHref: string): string | null {
  let source: URL
  try {
    source = new URL(rawHref, legacyOrigin)
  } catch {
    return null
  }

  const pathname = normalizeLegacyPath(source.pathname)
  if (pathname === '/login') {
    return buildTargetHref('/sign-in', source)
  }
  if (pathname === '/forbidden') {
    return buildTargetHref('/403', source)
  }

  const target = legacyConsoleRoutes[pathname]
  if (target) return buildTargetHref(target, source)
  if (pathname.startsWith('/console/')) {
    return buildTargetHref('/dashboard', source)
  }

  return null
}
