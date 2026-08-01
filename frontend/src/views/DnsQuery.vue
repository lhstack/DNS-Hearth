<template>
  <div class="dns-query">
    <!-- 页面标题 -->
    <div class="page-header">
      <div class="header-left">
        <h1>DNS 查询工具</h1>
        <p class="subtitle">测试 DNS 解析，支持多种记录类型查询</p>
      </div>
    </div>

    <!-- 查询表单 -->
    <el-card class="query-card" shadow="never">
      <div class="query-form">
        <div class="query-icon">
          <el-icon><Search /></el-icon>
        </div>
        <div class="query-inputs">
          <el-input
            v-model="queryForm.domain"
            placeholder="输入要查询的域名，如 example.com"
            size="large"
            clearable
            @keyup.enter="performQuery"
            class="domain-input"
          >
            <template #prefix>
              <el-icon><Link /></el-icon>
            </template>
          </el-input>
          <el-select
            v-model="queryForm.record_type"
            placeholder="选择记录类型"
            size="large"
            class="type-select"
          >
            <el-option
              v-for="type in recordTypes"
              :key="type.value"
              :label="type.label"
              :value="type.value"
            />
          </el-select>
          <el-button
            type="primary"
            size="large"
            @click="performQuery"
            :loading="querying"
            :disabled="!queryForm.domain"
            class="query-btn"
          >
            <el-icon><Search /></el-icon>
            查询
          </el-button>
        </div>
      </div>
    </el-card>

    <!-- 查询结果 -->
    <el-card v-if="result" class="result-card" shadow="never">
      <template #header>
        <div class="result-header">
          <div class="result-title">
            <el-icon><Document /></el-icon>
            <span>查询结果</span>
          </div>
          <div class="result-tags">
            <el-tag :type="getResponseCodeType(result.response_code)" effect="dark">
              {{ result.response_code }}
            </el-tag>
            <el-tag :type="result.cache_hit ? 'success' : 'info'" effect="plain">
              {{ result.cache_hit ? '缓存命中' : '缓存未命中' }}
            </el-tag>
            <el-tag v-if="result.rewrite_applied" type="warning" effect="plain">
              已重写
            </el-tag>
            <el-tag effect="plain">
              {{ result.response_time_ms }}ms
            </el-tag>
          </div>
        </div>
      </template>

      <!-- 响应元数据 -->
      <div class="metadata-grid">
        <div class="metadata-item">
          <span class="metadata-label">查询域名</span>
          <span class="metadata-value domain">{{ result.domain }}</span>
        </div>
        <div class="metadata-item">
          <span class="metadata-label">记录类型</span>
          <span class="metadata-value">
            <el-tag size="small">{{ result.record_type }}</el-tag>
          </span>
        </div>
        <div class="metadata-item">
          <span class="metadata-label">响应时间</span>
          <span class="metadata-value">{{ result.response_time_ms }} ms</span>
        </div>
        <div class="metadata-item">
          <span class="metadata-label">上游服务器</span>
          <span class="metadata-value">{{ result.upstream_used || '-' }}</span>
        </div>
      </div>

      <!-- DNS 记录表格 -->
      <div class="records-section" v-if="result.records.length > 0">
        <div class="section-title">
          <el-icon><List /></el-icon>
          <span>DNS 记录 ({{ result.records.length }})</span>
        </div>
        <el-table :data="result.records" stripe class="records-table">
          <el-table-column prop="name" label="名称" min-width="200">
            <template #default="{ row }">
              <span class="record-name">{{ row.name }}</span>
            </template>
          </el-table-column>
          <el-table-column prop="record_type" label="类型" width="100">
            <template #default="{ row }">
              <el-tag effect="dark" size="small">{{ row.record_type }}</el-tag>
            </template>
          </el-table-column>
          <el-table-column prop="value" label="值" min-width="280">
            <template #default="{ row }">
              <span class="record-value">{{ row.value }}</span>
            </template>
          </el-table-column>
          <el-table-column prop="ttl" label="TTL" width="100">
            <template #default="{ row }">
              <span class="ttl-value">{{ row.ttl }}s</span>
            </template>
          </el-table-column>
        </el-table>
      </div>

      <!-- 无记录 -->
      <el-empty v-else description="未找到 DNS 记录" :image-size="120" />
    </el-card>

    <!-- 错误显示 -->
    <el-card v-if="error" class="error-card" shadow="never">
      <el-result icon="error" title="查询失败" :sub-title="error">
        <template #extra>
          <el-button type="primary" @click="error = null" size="large">关闭</el-button>
        </template>
      </el-result>
    </el-card>

    <!-- 快捷查询 -->
    <el-card v-if="!result && !error" class="tips-card" shadow="never">
      <div class="tips-content">
        <div class="tips-icon">
          <el-icon><InfoFilled /></el-icon>
        </div>
        <div class="tips-text">
          <h4>快捷提示</h4>
          <ul>
            <li>输入域名后按 <kbd>Enter</kbd> 快速查询</li>
            <li>支持 A、AAAA、CNAME、MX、TXT 等多种记录类型</li>
            <li>查询结果会显示是否命中缓存及响应时间</li>
          </ul>
        </div>
      </div>
      <div class="quick-queries">
        <span class="quick-label">快捷查询：</span>
        <el-button
          v-for="domain in quickDomains"
          :key="domain"
          size="small"
          @click="quickQuery(domain)"
        >
          {{ domain }}
        </el-button>
      </div>
    </el-card>
  </div>
</template>

<script setup lang="ts">
import {reactive, ref} from 'vue'
import {ElMessage} from 'element-plus'
import {Document, InfoFilled, Link, List, Search} from '@element-plus/icons-vue'
import api from '../api'

interface DnsRecord {
  name: string
  record_type: string
  value: string
  ttl: number
}

interface QueryResult {
  domain: string
  record_type: string
  records: DnsRecord[]
  response_time_ms: number
  cache_hit: boolean
  upstream_used: string | null
  rewrite_applied: boolean
  response_code: string
}

const recordTypes = [
  { value: 'A', label: 'A - IPv4 地址' },
  { value: 'AAAA', label: 'AAAA - IPv6 地址' },
  { value: 'CNAME', label: 'CNAME - 别名记录' },
  { value: 'MX', label: 'MX - 邮件服务器' },
  { value: 'TXT', label: 'TXT - 文本记录' },
  { value: 'PTR', label: 'PTR - 反向解析' },
  { value: 'NS', label: 'NS - 域名服务器' },
  { value: 'SOA', label: 'SOA - 授权起始' },
  { value: 'SRV', label: 'SRV - 服务定位' }
]

const quickDomains = ['google.com', 'github.com', 'cloudflare.com', 'baidu.com']

const queryForm = reactive({
  domain: '',
  record_type: 'A'
})

const querying = ref(false)
const result = ref<QueryResult | null>(null)
const error = ref<string | null>(null)

function getResponseCodeType(code: string): string {
  if (code === 'NOERROR') return 'success'
  if (code === 'NXDOMAIN') return 'warning'
  return 'danger'
}

function quickQuery(domain: string) {
  queryForm.domain = domain
  performQuery()
}

async function performQuery() {
  if (!queryForm.domain) {
    ElMessage.warning('请输入域名')
    return
  }

  querying.value = true
  result.value = null
  error.value = null

  try {
    const response = await api.post('/api/dns/query', queryForm)
    result.value = response.data
  } catch (err: any) {
    error.value = err.response?.data?.message || '查询失败'
  } finally {
    querying.value = false
  }
}
</script>

<style scoped>
.dns-query {
  max-width: 1200px;
  margin: 0 auto;
}

/* 页面标题 */
.page-header {
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

/* 查询卡片 */
.query-card {
  border-radius: 12px;
  border: none;
  margin-bottom: 24px;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
}

.query-card :deep(.el-card__body) {
  padding: 32px;
}

.query-form {
  display: flex;
  align-items: center;
  gap: 24px;
}

.query-icon {
  width: 64px;
  height: 64px;
  background: rgba(255, 255, 255, 0.2);
  border-radius: 16px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  font-size: 32px;
  flex-shrink: 0;
}

.query-inputs {
  flex: 1;
  display: flex;
  gap: 12px;
  align-items: center;
}

.domain-input {
  flex: 1;
}

.domain-input :deep(.el-input__wrapper) {
  background: rgba(255, 255, 255, 0.95);
  border-radius: 8px;
}

.type-select {
  width: 200px;
}

.type-select :deep(.el-input__wrapper) {
  background: rgba(255, 255, 255, 0.95);
  border-radius: 8px;
}

.query-btn {
  padding: 0 24px;
}

/* 结果卡片 */
.result-card {
  border-radius: 12px;
  border: none;
  margin-bottom: 24px;
}

.result-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  flex-wrap: wrap;
  gap: 12px;
}

.result-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 16px;
  font-weight: 600;
  color: #303133;
}

.result-title .el-icon {
  color: #667eea;
}

.result-tags {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  align-items: center;
}

/* 元数据网格 */
.metadata-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 16px;
  padding: 20px;
  background: #f8f9fa;
  border-radius: 8px;
  margin-bottom: 24px;
}

.metadata-item {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.metadata-label {
  font-size: 12px;
  color: #909399;
}

.metadata-value {
  font-size: 14px;
  font-weight: 500;
  color: #303133;
}

.metadata-value.domain {
  font-family: 'Monaco', 'Menlo', monospace;
  color: #667eea;
}

/* 记录区域 */
.records-section {
  margin-top: 24px;
}

.section-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 15px;
  font-weight: 600;
  color: #303133;
  margin-bottom: 16px;
}

.section-title .el-icon {
  color: #667eea;
}

.records-table :deep(.el-table__header th) {
  background: #f8f9fa;
  color: #606266;
  font-weight: 600;
}

.record-name {
  font-family: 'Monaco', 'Menlo', monospace;
  font-size: 13px;
  color: #303133;
}

.record-value {
  font-family: 'Monaco', 'Menlo', monospace;
  font-size: 13px;
  color: #606266;
  word-break: break-all;
}

.ttl-value {
  color: #909399;
}

/* 错误卡片 */
.error-card {
  border-radius: 12px;
  border: none;
  margin-bottom: 24px;
}

/* 提示卡片 */
.tips-card {
  border-radius: 12px;
  border: none;
}

.tips-content {
  display: flex;
  gap: 16px;
  margin-bottom: 20px;
}

.tips-icon {
  width: 48px;
  height: 48px;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  border-radius: 12px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  font-size: 24px;
  flex-shrink: 0;
}

.tips-text h4 {
  margin: 0 0 8px 0;
  font-size: 15px;
  font-weight: 600;
  color: #303133;
}

.tips-text ul {
  margin: 0;
  padding-left: 20px;
  color: #606266;
  font-size: 14px;
  line-height: 1.8;
}

.tips-text kbd {
  background: #f0f0f0;
  padding: 2px 6px;
  border-radius: 4px;
  font-size: 12px;
  border: 1px solid #ddd;
}

.quick-queries {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  padding-top: 16px;
  border-top: 1px solid #f0f0f0;
}

.quick-label {
  font-size: 13px;
  color: #909399;
}

/* 响应式 */
@media (max-width: 768px) {
  .query-form {
    flex-direction: column;
  }

  .query-icon {
    display: none;
  }

  .query-inputs {
    flex-direction: column;
    width: 100%;
  }

  .domain-input,
  .type-select {
    width: 100%;
  }

  .query-btn {
    width: 100%;
  }

  .result-header {
    flex-direction: column;
    align-items: flex-start;
  }
}
</style>
