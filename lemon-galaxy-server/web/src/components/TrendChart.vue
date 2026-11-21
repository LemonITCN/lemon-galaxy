<script lang="ts">
let chartSeq = 0

export interface ChartPoint {
  label: string
  value: number
  /** 悬停时显示的完整标签（默认同 label） */
  title?: string
}
</script>

<script setup lang="ts">
// 轻量趋势图（自绘 SVG）：柱状或面积折线，带悬停数值。不引入图表库。
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'

const props = withDefaults(
  defineProps<{
    points: ChartPoint[]
    type?: 'bar' | 'area'
    color?: string
    height?: number
    format?: (n: number) => string
    /** 数值是字节数：刻度按 1024 进位取整（4 GB 而不是 4.7 GB） */
    binary?: boolean
  }>(),
  { type: 'bar', color: 'var(--lg-chart-1)', height: 150, format: (n: number) => String(n), binary: false },
)

chartSeq += 1
const gradId = `lg-chart-grad-${chartSeq}`

const wrap = ref<HTMLDivElement>()
const width = ref(0)
let ro: ResizeObserver | undefined

onMounted(() => {
  if (!wrap.value) return
  width.value = wrap.value.clientWidth
  ro = new ResizeObserver((entries) => {
    width.value = Math.floor(entries[0].contentRect.width)
  })
  ro.observe(wrap.value)
})
onBeforeUnmount(() => ro?.disconnect())

const PAD_L = 46
const PAD_R = 8
const PAD_T = 10
const PAD_B = 24

function niceMax(v: number): number {
  if (v <= 0) return 1
  const exp = Math.floor(Math.log10(v))
  const base = 10 ** exp
  const f = v / base
  const nice = f <= 1 ? 1 : f <= 2 ? 2 : f <= 2.5 ? 2.5 : f <= 5 ? 5 : 10
  return nice * base
}

const max = computed(() => {
  const top = Math.max(0, ...props.points.map((p) => p.value))
  // 字节图至少显示到 1 KB，避免出现 0.5 B 这样的刻度
  if (props.binary && top < 1024) return 1024
  if (!props.binary) return niceMax(top)
  const unit = 1024 ** Math.floor(Math.log(top) / Math.log(1024))
  return niceMax(top / unit) * unit
})
const plotW = computed(() => Math.max(0, width.value - PAD_L - PAD_R))
const plotH = computed(() => props.height - PAD_T - PAD_B)
const step = computed(() => (props.points.length ? plotW.value / props.points.length : 0))

function y(v: number) {
  return PAD_T + plotH.value - (v / max.value) * plotH.value
}
function cx(i: number) {
  return PAD_L + step.value * i + step.value / 2
}

const grid = computed(() => [0, 0.5, 1].map((f) => ({ y: y(max.value * f), label: props.format(max.value * f) })))

const bars = computed(() => {
  const bw = Math.max(2, Math.min(28, step.value * 0.56))
  return props.points.map((p, i) => {
    const top = y(p.value)
    const h = Math.max(p.value > 0 ? 2 : 0, PAD_T + plotH.value - top)
    return { x: cx(i) - bw / 2, y: PAD_T + plotH.value - h, w: bw, h }
  })
})

const linePath = computed(() => props.points.map((p, i) => `${i ? 'L' : 'M'}${cx(i).toFixed(1)},${y(p.value).toFixed(1)}`).join(' '))
const areaPath = computed(() => {
  if (!props.points.length) return ''
  const base = (PAD_T + plotH.value).toFixed(1)
  return `${linePath.value} L${cx(props.points.length - 1).toFixed(1)},${base} L${cx(0).toFixed(1)},${base} Z`
})

/** X 轴标签太密时隔几个显示一个 */
const labelEvery = computed(() => {
  const n = props.points.length
  if (!n || !step.value) return 1
  return Math.max(1, Math.ceil(46 / step.value))
})

const hover = ref<number | null>(null)
function onMove(e: MouseEvent) {
  if (!wrap.value || !props.points.length) return
  const rect = wrap.value.getBoundingClientRect()
  const x = e.clientX - rect.left - PAD_L
  const i = Math.floor(x / step.value)
  hover.value = i >= 0 && i < props.points.length ? i : null
}

const tip = computed(() => {
  if (hover.value === null) return null
  const p = props.points[hover.value]
  const left = cx(hover.value)
  const alignRight = left > width.value - 120
  return { p, left, alignRight, top: Math.max(0, y(p.value) - 8) }
})
</script>

<template>
  <div ref="wrap" class="chart" :style="{ height: height + 'px' }" @mousemove="onMove" @mouseleave="hover = null">
    <svg v-if="width > 0" :width="width" :height="height" class="svg">
      <defs>
        <linearGradient :id="gradId" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" :stop-color="color" stop-opacity="0.28" />
          <stop offset="1" :stop-color="color" stop-opacity="0.02" />
        </linearGradient>
      </defs>
      <g class="grid">
        <g v-for="g in grid" :key="g.y">
          <line :x1="PAD_L" :x2="width - PAD_R" :y1="g.y" :y2="g.y" />
          <text :x="PAD_L - 8" :y="g.y + 4" text-anchor="end">{{ g.label }}</text>
        </g>
      </g>
      <template v-if="type === 'bar'">
        <rect
          v-for="(b, i) in bars"
          :key="i"
          :x="b.x"
          :y="b.y"
          :width="b.w"
          :height="b.h"
          rx="3"
          :fill="color"
          :opacity="hover === null || hover === i ? 1 : 0.45"
        />
      </template>
      <template v-else>
        <path :d="areaPath" :fill="`url(#${gradId})`" />
        <path :d="linePath" fill="none" :stroke="color" stroke-width="2" stroke-linejoin="round" stroke-linecap="round" />
        <line v-if="hover !== null" class="cursor" :x1="cx(hover)" :x2="cx(hover)" :y1="PAD_T" :y2="PAD_T + plotH" />
        <circle v-if="hover !== null" :cx="cx(hover)" :cy="y(points[hover].value)" r="4" :fill="color" class="dot" />
      </template>
      <g class="xlabels">
        <template v-for="(p, i) in points" :key="'x' + i">
          <text v-if="i % labelEvery === 0" :x="cx(i)" :y="height - 6" text-anchor="middle">{{ p.label }}</text>
        </template>
      </g>
    </svg>
    <div
      v-if="tip"
      class="tip"
      :class="{ right: tip.alignRight }"
      :style="{ left: tip.left + 'px', top: tip.top + 'px' }"
    >
      <div class="tip-label">{{ tip.p.title ?? tip.p.label }}</div>
      <div class="tip-value">{{ format(tip.p.value) }}</div>
    </div>
  </div>
</template>

<style scoped>
.chart {
  position: relative;
  width: 100%;
  min-width: 0;
}
.svg {
  display: block;
  overflow: visible;
}
.grid line {
  stroke: var(--lg-chart-grid);
  stroke-width: 1;
}
.grid text,
.xlabels text {
  fill: var(--lg-text-3);
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}
.cursor {
  stroke: var(--lg-border);
  stroke-dasharray: 3 3;
}
.dot {
  stroke: var(--lg-surface);
  stroke-width: 2;
}
.tip {
  position: absolute;
  transform: translate(-50%, -100%);
  pointer-events: none;
  background: var(--lg-surface);
  border: 1px solid var(--lg-border);
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.12);
  border-radius: 7px;
  padding: 5px 9px;
  white-space: nowrap;
  z-index: 2;
}
.tip.right {
  transform: translate(-100%, -100%);
}
.tip-label {
  font-size: 11.5px;
  color: var(--lg-text-3);
}
.tip-value {
  font-size: 13px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}
</style>
