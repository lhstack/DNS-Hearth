import { createRouter, createWebHistory } from 'vue-router'
import type { RouteRecordRaw } from 'vue-router'

const routes: RouteRecordRaw[] = [
  {
    path: '/',
    component: () => import('../layouts/MainLayout.vue'),

    children: [
      {
        path: '',
        name: 'Dashboard',
        component: () => import('../views/Dashboard.vue')
      },
      {
        path: 'records',
        name: 'DnsRecords',
        component: () => import('../views/DnsRecords.vue')
      },
      {
        path: 'rewrite',
        name: 'RewriteRules',
        component: () => import('../views/RewriteRules.vue')
      },
      {
        path: 'upstreams',
        name: 'Upstreams',
        component: () => import('../views/Upstreams.vue')
      },
      {
        path: 'cache',
        name: 'Cache',
        component: () => import('../views/Cache.vue')
      },
      {
        path: 'query',
        name: 'DnsQuery',
        component: () => import('../views/DnsQuery.vue')
      },
      {
        path: 'logs',
        name: 'QueryLogs',
        component: () => import('../views/QueryLogs.vue')
      },
      {
        path: 'settings',
        name: 'Settings',
        component: () => import('../views/Settings.vue')
      }
    ]
  }
]

const router = createRouter({
  history: createWebHistory(),
  routes
})


export default router
