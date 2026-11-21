package cn.lemonit.galaxy.ui.remote

import android.media.MediaCodec
import android.media.MediaFormat
import android.os.Build
import android.util.Log
import android.view.Surface
import cn.lemonit.galaxy.core.bridge.VideoCodec
import cn.lemonit.galaxy.core.bridge.VideoSink
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.TimeUnit

/**
 * 远程画面硬件解码：内核推来的 Annex B 码流（H.264 / H.265）交给 MediaCodec，直接输出到 SurfaceView 的 Surface。
 *
 * - 解码在专用线程上进行；输入队列满时丢最旧的非关键帧，避免延迟累积。
 * - Surface 未就绪或解码器重建后，等下一个关键帧再开始送数据（并向内核要关键帧）。
 * - 画面尺寸 / 编码变化时（onConfig）重建解码器。
 */
class VideoDecoder(
    private val requestKeyframe: () -> Unit,
    private val onVideoSize: (Int, Int) -> Unit,
    private val onCursorChanged: (Int, Int, Boolean) -> Unit,
) : VideoSink {
    private class Frame(val data: ByteArray, val ptsUs: Long, val keyframe: Boolean)

    private val queue = ArrayBlockingQueue<Frame>(32)
    @Volatile private var surface: Surface? = null
    @Volatile private var codecType: VideoCodec? = null
    @Volatile private var width = 0
    @Volatile private var height = 0
    @Volatile private var reconfigure = false
    @Volatile private var running = true
    private var codec: MediaCodec? = null
    private var waitingKeyframe = true
    private val thread = Thread(::loop, "galaxy-video-decoder").apply { isDaemon = true; start() }

    fun setSurface(s: Surface?) {
        surface = s
        reconfigure = true
        if (s != null) requestKeyframe()
    }

    fun release() {
        running = false
        thread.interrupt()
    }

    override fun onConfig(codec: VideoCodec, width: UInt, height: UInt) {
        codecType = codec
        this.width = width.toInt()
        this.height = height.toInt()
        reconfigure = true
        onVideoSize(this.width, this.height)
    }

    override fun onFrame(data: ByteArray, ptsUs: ULong, keyframe: Boolean) {
        val f = Frame(data, ptsUs.toLong(), keyframe)
        if (!queue.offer(f)) {
            // 积压：清空队列，等关键帧
            queue.clear()
            if (keyframe) queue.offer(f) else requestKeyframe()
        }
    }

    override fun onCursor(x: Int, y: Int, visible: Boolean) = onCursorChanged(x, y, visible)

    private fun mime(c: VideoCodec) = when (c) {
        VideoCodec.H264 -> MediaFormat.MIMETYPE_VIDEO_AVC
        VideoCodec.H265 -> MediaFormat.MIMETYPE_VIDEO_HEVC
    }

    private fun rebuild() {
        reconfigure = false
        codec?.let { runCatching { it.stop() }; runCatching { it.release() } }
        codec = null
        val s = surface ?: return
        val type = codecType ?: return
        if (width <= 0 || height <= 0) return
        try {
            val format = MediaFormat.createVideoFormat(mime(type), width, height)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) format.setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
            format.setInteger(MediaFormat.KEY_PRIORITY, 0)
            val c = MediaCodec.createDecoderByType(mime(type))
            c.configure(format, s, null, 0)
            c.start()
            codec = c
            waitingKeyframe = true
            requestKeyframe()
        } catch (e: Exception) {
            Log.e(TAG, "decoder setup failed", e)
            codec = null
        }
    }

    private fun loop() {
        val info = MediaCodec.BufferInfo()
        while (running) {
            try {
                if (reconfigure) rebuild()
                val c = codec
                val f = queue.poll(10, TimeUnit.MILLISECONDS)
                if (c == null) continue
                if (f != null) {
                    if (waitingKeyframe && !f.keyframe) {
                        // 丢掉关键帧之前的数据
                    } else {
                        waitingKeyframe = false
                        val idx = c.dequeueInputBuffer(10_000)
                        if (idx >= 0) {
                            val buf = c.getInputBuffer(idx)!!
                            buf.clear()
                            buf.put(f.data, 0, minOf(f.data.size, buf.remaining()))
                            c.queueInputBuffer(idx, 0, buf.position(), f.ptsUs, if (f.keyframe) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0)
                        }
                    }
                }
                // 取出所有已解码的帧并立即显示（低延迟：不按时间戳排队）
                while (true) {
                    val out = c.dequeueOutputBuffer(info, 0)
                    if (out >= 0) {
                        c.releaseOutputBuffer(out, true)
                    } else {
                        break
                    }
                }
            } catch (e: InterruptedException) {
                break
            } catch (e: Exception) {
                Log.w(TAG, "decode error: ${e.message}")
                reconfigure = true
            }
        }
        codec?.let { runCatching { it.stop() }; runCatching { it.release() } }
        codec = null
    }

    private companion object {
        const val TAG = "VideoDecoder"
    }
}
