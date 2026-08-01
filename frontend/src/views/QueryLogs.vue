<template>
  <div class="query-logs">
    <!-- 页面标题 -->
    <div class="page-header">
      <div class="header-left">
        <h1>查询日志</h1>
        <p class="subtitle">查看 DNS 查询历史记录，分析查询统计数据</p>
      </div>
      <el-button type="primary" size="large" @click="fetchLogs">
        <el-icon><Refresh /></el-icon>
        刷新
      </el-button>
    </div>

    <!-- 统计卡片 -->
    <el-row :gutter="20" class="stats-row">
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);">
            <el-icon><DataAnalysis /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ stats.total_queries }}</span>
            <span class="stat-label">总查询</span>
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: linear-gradient(135deg, #11998e 0%, #38ef7d 100%);">
            <el-icon><Calendar /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ stats.queries_today }}</span>
            <span class="stat-label">今日</span>
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: linear-gradient(135deg, #f5576c 0%, #f093fb 100%);">
            <el-icon><CircleCheck /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ stats.cache_hits }}</span>
            <span class="stat-label">命中</span>
          </div>
        </div>
      </el-col>
      <el-col :xs="12" :sm="6">
        <div class="stat-card">
          <div class="stat-icon" style="background: linear-gradient(135deg, #4facfe 0%, #00f2fe 100%);">
            <el-icon><TrendCharts /></el-icon>
          </div>
          <div class="stat-info">
            <span class="stat-value">{{ (stats.cache_hit_rate * 100).toFixed(1) }}%</span>
            <span class="stat-label">缓存命中率</span>
          </div>
        </div>
      </el-col>
    </el-row>

    <!-- 筛选器 -->
    <el-card class="filter-card" shadow="never">
      <div class="filter-form">
        <div class="filter-item">
          <label>域名</label>
          <el-input
            v-model="filters.query_name"
            placeholder="搜索域名"
            clearable
            @clear="fetchLogs"
            @keyup.enter="fetchLogs"
            size="large"
          >
            <template #prefix>
              <el-icon><Search /></el-icon>
            </template>
          </el-input>
        </div>
        <div class="filter-item">
          <label>类型</label>
          <el-select
            v-model="filters.query_type"
            placeholder="全部类型"
            clearable
            size="large"
            @change="fetchLogs"
          >
            <el-option
              v-for="type in recordTypes"
              :key="type"
              :label="type"
              :value="type"
            />
          </el-select>
        </div>
        <div class="filter-item">
          <label>客户端 IP</label>
          <el-input
            v-model="filters.client_ip"
            placeholder="搜索 IP"
            clearable
            @clear="fetchLogs"
            @keyup.enter="fetchLogs"
            size="large"
          >
            <template #prefix>
              <el-icon><Monitor /></el-icon>
            </template>
          </el-input>
        </div>
        <div class="filter-item">
          <label>缓存命中</label>
          <el-select
            v-model="filters.cache_hit"
            placeholder="全部"
            clearable
            size="large"
            @change="fetchLogs"
          >
            <el-option label="命中" :value="true" />
            <el-option label="未命中" :value="false" />
          </el-select>
        </div>
        <div class="filter-item" style="min-width: 320px;">
          <label>时间范围</label>
          <el-date-picker
            v-model="filters.dateRange"
            type="datetimerange"
            range-separator="至"
            start-placeholder="开始时间"
            end-placeholder="结束时间"
            :shortcuts="shortcuts"
            size="large"
            @change="fetchLogs"
          />
        </div>
        <div class="filter-actions">
          <el-button type="primary" @click="fetchLogs" size="large">
            <el-icon><Search /></el-icon>
            搜索
          </el-button>
          <el-button @click="resetFilters" size="large">
            <el-icon><RefreshRight /></el-icon>
            重置
          </el-button>

          <el-dropdown @command="handleExport" trigger="click">
            <el-button type="success" size="large">
              <el-icon><Download /></el-icon>
              导出
              <el-icon class="el-icon--right"><ArrowDown /></el-icon>
            </el-button>
            <template #dropdown>
              <el-dropdown-menu>
                <el-dropdown-item command="csv">导出 CSV</el-dropdown-item>
                <el-dropdown-item command="json">导出 JSON</el-dropdown-item>
              </el-dropdown-menu>
            </template>
          </el-dropdown>
        </div>
      </div>
    </el-card>

    <!-- 日志表格 -->
    <el-card class="table-card" shadow="never">
      <div class="table-wrapper">
        <el-table :data="logs" v-loading="loading" stripe class="custom-table">
          <el-table-column prop="id" label="ID" width="100" show-overflow-tooltip class-name="hidden-xs-only" />
        <el-table-column prop="query_name" label="查询域名" min-width="200" show-overflow-tooltip>
          <template #default="{ row }">
            <span class="domain-name">{{ row.query_name }}</span>
          </template>
        </el-table-column>
        <el-table-column prop="query_type" label="类型" width="90">
          <template #default="{ row }">
            <el-tag effect="dark" size="small">{{ row.query_type }}</el-tag>
          </template>
        </el-table-column>
          <el-table-column prop="client_ip" label="客户端" width="130" class-name="hidden-xs-only">
            <template #default="{ row }">
              <span class="client-ip">{{ row.client_ip }}</span>
            </template>
          </el-table-column>
        <el-table-column prop="response_code" label="响应码" width="110" show-overflow-tooltip>
          <template #default="{ row }">
            <el-tag :type="getResponseCodeType(row.response_code)" size="small" effect="plain">
              {{ row.response_code || '-' }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="response_time" label="响应时间" width="100">
          <template #default="{ row }">
            <span class="response-time">{{ row.response_time ? `${row.response_time}ms` : '-' }}</span>
          </template>
        </el-table-column>
          <el-table-column prop="cache_hit" label="缓存" width="70">
            <template #default="{ row }">
              <el-tag :type="row.cache_hit ? 'success' : 'info'" size="small" effect="plain">
                {{ row.cache_hit ? '是' : '否' }}
              </el-tag>
            </template>
          </el-table-column>
          <el-table-column prop="upstream_used" label="上游服务器" min-width="150" class-name="hidden-xs-only" show-overflow-tooltip>
            <template #default="{ row }">
              <span class="upstream-name">{{ row.upstream_used || '-' }}</span>
            </template>
          </el-table-column>
        <el-table-column prop="created_at" label="时间" width="180">
          <template #default="{ row }">
            <span class="time-value">{{ formatTime(row.created_at) }}</span>
          </template>
        </el-table-column>
        <template #empty>
          <el-empty description="暂无查询日志" />
        </template>
      </el-table>
      </div>

      <div class="pagination-container">
        <el-pagination
          v-model:current-page="currentPage"
          v-model:page-size="pageSize"
          :page-sizes="[20, 50, 100]"
          :total="total"
          layout="total, sizes, prev, pager, next"
          @size-change="handleSizeChange"
          @current-change="handlePageChange"
        />
      </div>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, onMounted, computed } from 'vue'
import { ElMessage } from 'element-plus'
import {
  Refresh, DataAnalysis, Calendar, CircleCheck, TrendCharts,
  Search, Monitor, RefreshRight, Download, ArrowDown
} from '@element-plus/icons-vue'
import api from '../api'

interface QueryLog {
  id: number
  query_name: string
  query_type: string
  client_ip: string
  response_code: string | null
  response_time: number | null
  cache_hit: boolean
  upstream_used: string | null
  created_at: string
}

interface QueryStats {
  total_queries: number
  cache_hits: number
  queries_today: number
  cache_hit_rate: number
}

const recordTypes = ['A', 'AAAA', 'CNAME', 'MX', 'TXT', 'PTR', 'NS', 'SOA', 'SRV']

const logs = ref<QueryLog[]>([])
const loading = ref(false)
const total = ref(0)
const currentPage = ref(1)
const pageSize = ref(20)

const stats = ref<QueryStats>({
  total_queries: 0,
  cache_hits: 0,
  queries_today: 0,
  cache_hit_rate: 0
})

const filters = reactive({
  query_name: '',
  query_type: null as string | null,
  client_ip: '',
  cache_hit: null as boolean | null,
  dateRange: null as [Date, Date] | null
})

const offset = computed(() => (currentPage.value - 1) * pageSize.value)

function getResponseCodeType(code: string | null): string {
  if (!code) return 'info'
  if (code === 'NOERROR') return 'success'
  if (code === 'NXDOMAIN') return 'warning'
  return 'danger'
}

function formatTime(dateStr: string): string {
  const date = new Date(dateStr)
  return date.toLocaleString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit'
  })
}

async function fetchLogs() {
  loading.value = true
  try {
    const params: Record<string, any> = {
      limit: pageSize.value,
      offset: offset.value
    }

    if (filters.query_name) params.query_name = filters.query_name
    if (filters.query_type) params.query_type = filters.query_type
    if (filters.client_ip) params.client_ip = filters.client_ip
    if (filters.cache_hit !== null) params.cache_hit = filters.cache_hit
    if (filters.dateRange) {
      params.start_time = filters.dateRange[0].toISOString()
      params.end_time = filters.dateRange[1].toISOString()
    }

    const response = await api.get('/api/logs', { params })
    logs.value = response.data.data
    total.value = response.data.total
  } catch (error: any) {
    ElMessage.error(error.response?.data?.message || '获取日志失败')
  } finally {
    loading.value = false
  }
}

async function handleExport(format: string) {
  const params: string[] = []
  if (filters.query_name) params.push(`query_name=${encodeURIComponent(filters.query_name)}`)
  if (filters.query_type) params.push(`query_type=${encodeURIComponent(filters.query_type)}`)
  if (filters.client_ip) params.push(`client_ip=${encodeURIComponent(filters.client_ip)}`)
  if (filters.cache_hit !== null) params.push(`cache_hit=${filters.cache_hit}`)
  if (filters.dateRange) {
    params.push(`start_time=${encodeURIComponent(filters.dateRange[0].toISOString())}`)
    params.push(`end_time=${encodeURIComponent(filters.dateRange[1].toISOString())}`)
  }
  params.push(`format=${format}`)

  try {
    const response = await api.get<string>('/api/logs/export', { params: { ...Object.fromEntries(params.map(item => item.split('=').map(decodeURIComponent))), format } })
    const blob = new Blob([response.data], { type: format === 'csv' ? 'text/csv;charset=utf-8' : 'application/json' })
    const url = URL.createObjectURL(blob)
    const link = document.createElement('a')
    link.href = url
    link.download = `dns-query-logs.${format === 'csv' ? 'csv' : 'json'}`
    link.click()
    URL.revokeObjectURL(url)
  } catch (error: any) {
    ElMessage.error(error?.message || '导出日志失败')
  }
}

const shortcuts = [
  {
    text: '最近1小时',
    value: () => {
      const end = new Date()
      const start = new Date()
      start.setTime(start.getTime() - 3600 * 1000)
      return [start, end]
    },
  },
  {
    text: '最近24小时',
    value: () => {
      const end = new Date()
      const start = new Date()
      start.setTime(start.getTime() - 3600 * 1000 * 24)
      return [start, end]
    },
  },
  {
    text: '最近7天',
    value: () => {
      const end = new Date()
      const start = new Date()
      start.setTime(start.getTime() - 3600 * 1000 * 24 * 7)
      return [start, end]
    },
  },
]

async function fetchStats() {
  try {
    const response = await api.get('/api/logs/stats')
    const value = response.data as QueryStats
    if (!Number.isFinite(value.cache_hit_rate)) {
      throw new Error('查询统计返回了无效的缓存命中率')
    }
    stats.value = value
  } catch (error: any) {
    ElMessage.error(error?.message || '获取查询统计失败')
  }
}

function resetFilters() {
  filters.query_name = ''
  filters.query_type = null
  filters.client_ip = ''
  filters.cache_hit = null
  filters.dateRange = null
  currentPage.value = 1
  fetchLogs()
}

function handleSizeChange() {
  currentPage.value = 1
  fetchLogs()
}

function handlePageChange() {
  fetchLogs()
}

onMounted(() => {
  fetchLogs()
  fetchStats()
})
</script>

<style scoped>
.query-logs {
  max-width: 1400px;
  margin: 0 auto;
}

/* 页面标题 */
.page-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  margin-bottom: 24px;
}

.header-left h1 {
  margin: 0 0 8px 0;
  font-size: 24px;
  font-weight: 600;
  color: #303133;
}

.subtitle {
  margin: 0;
  font-size: 14px;
  color: #909399;
}

/* 统计卡片 */
.stats-row {
  margin-bottom: 24px;
}

.stat-card {
  background: #fff;
  border-radius: 12px;
  padding: 20px;
  display: flex;
  align-items: center;
  gap: 16px;
  box-shadow: 0 2px 12px rgba(0, 0, 0, 0.04);
  transition: transform 0.3s, box-shadow 0.3s;
}

.stat-card:hover {
  transform: translateY(-2px);
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.08);
}

.stat-icon {
  width: 48px;
  height: 48px;
  border-radius: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  font-size: 24px;
}

.stat-info {
  display: flex;
  flex-direction: column;
}

.stat-value {
  font-size: 24px;
  font-weight: 600;
  color: #303133;
}

.stat-label {
  font-size: 13px;
  color: #909399;
  margin-top: 4px;
}

/* 筛选卡片 */
.filter-card {
  border-radius: 12px;
  border: none;
  margin-bottom: 24px;
}

.filter-form {
  display: flex;
  flex-wrap: wrap;
  gap: 16px;
  align-items: flex-end;
}

.filter-item {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 180px;
}

.filter-item label {
  font-size: 13px;
  color: #606266;
  font-weight: 500;
}

.filter-actions {
  display: flex;
  gap: 8px;
  margin-left: auto;
}

/* 表格卡片 */
.table-card {
  border-radius: 12px;
  border: none;
}

.table-card :deep(.el-card__body) {
  padding: 0;
}

.custom-table :deep(.el-table__header th) {
  background: #f8f9fa;
  color: #606266;
  font-weight: 600;
}

.domain-name {
  font-family: 'Monaco', 'Menlo', monospace;
  font-size: 13px;
  color: #303133;
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.client-ip {
  font-family: 'Monaco', 'Menlo', monospace;
  font-size: 13px;
  color: #606266;
}

.response-time {
  color: #909399;
}

.upstream-name {
  font-size: 13px;
  color: #606266;
}

.time-value {
  font-size: 13px;
  color: #909399;
}

.pagination-container {
  display: flex;
  justify-content: flex-end;
  padding: 16px 20px;
  border-top: 1px solid #f0f0f0;
}

/* 表格包装器 */
.table-wrapper {
  overflow-x: auto;
  -webkit-overflow-scrolling: touch;
}

/* 响应式 */
@media (max-width: 768px) {
  .page-header {
    flex-direction: column;
    align-items: stretch;
    gap: 16px;
  }

  .header-left h1 {
    font-size: 20px;
  }

  .stat-card {
    padding: 12px;
    gap: 10px;
  }

  .stat-icon {
    width: 36px;
    height: 36px;
    font-size: 18px;
    border-radius: 8px;
  }

  .stat-value {
    font-size: 18px;
  }

  .stat-label {
    font-size: 12px;
  }

  .filter-form {
    flex-direction: column;
    gap: 12px;
  }

  .filter-item {
    width: 100%;
    min-width: unset;
  }

  .filter-actions {
    width: 100%;
    margin-left: 0;
    gap: 12px;
  }

  .filter-actions .el-button {
    flex: 1;
    margin-left: 0;
  }

  .pagination-container {
    justify-content: center;
    padding: 12px;
  }

  .pagination-container :deep(.el-pagination) {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
  }

  .pagination-container :deep(.el-pagination__total),
  .pagination-container :deep(.el-pagination__sizes) {
    display: none;
  }
}
</style>
