package com.termux.terminal;

import android.annotation.SuppressLint;
import android.os.Handler;
import android.os.Looper;
import android.os.Message;

import java.nio.charset.StandardCharsets;
import java.util.UUID;

/**
 * A terminal session, coupling a terminal emulator to a byte stream.
 * <p>
 * LemonGalaxy 修改说明（相对 termux-app v0.118.3 原版，按 Apache-2.0 第 4 条标注）：
 * 原版在本机 fork 子进程并通过伪终端读写；LemonGalaxy 的终端在远端被控设备上，字节流经内核端到端通道传输，
 * 所以这里去掉了 JNI 子进程、伪终端与读写线程，改为：
 * <ul>
 *   <li>构造时传入 {@link Transport}：用户输入经 {@link Transport#write} 交给内核，尺寸变化经 {@link Transport#resize} 通知远端；</li>
 *   <li>远端输出由 {@link #appendRemoteOutput(byte[], int, int)} 喂入（任意线程），在主线程交给仿真器；</li>
 *   <li>远端会话结束时调用 {@link #onRemoteExit(Integer)}。</li>
 * </ul>
 * 其余（仿真、回调、编码）保持原样。
 */
public final class TerminalSession extends TerminalOutput {

    /** 远端字节通道（LemonGalaxy 增补）。 */
    public interface Transport {
        /** 把用户输入（已编码为终端字节）发往远端。 */
        void write(byte[] data, int offset, int count);

        /** 终端尺寸变化，通知远端伪终端。 */
        void resize(int columns, int rows);

        /** 用户要求结束会话。 */
        void close();
    }

    private static final int MSG_NEW_INPUT = 1;
    private static final int MSG_PROCESS_EXITED = 4;

    public final String mHandle = UUID.randomUUID().toString();

    TerminalEmulator mEmulator;

    /**
     * A queue written to from a separate thread when the remote outputs, and read by main thread to process by
     * terminal emulator.
     */
    final ByteQueue mProcessToTerminalIOQueue = new ByteQueue(64 * 1024);
    /** Buffer to write translate code points into utf8 before writing to the transport */
    private final byte[] mUtf8InputBuffer = new byte[5];

    /** Callback which gets notified when a session finishes or changes title. */
    TerminalSessionClient mClient;

    /** 是否仍在运行（远端未退出）。 */
    private boolean mRunning = true;

    /** The exit status of the remote shell. Only valid if not running. */
    int mShellExitStatus;

    /** Set by the application for user identification of session, not by terminal. */
    public String mSessionName;

    final Handler mMainThreadHandler = new MainThreadHandler();

    private final Transport mTransport;
    private final Integer mTranscriptRows;

    /** 视图尺寸确定之前远端已经到达的字节，仿真器建立后一次性补喂。 */
    private final java.io.ByteArrayOutputStream mPendingBeforeEmulator = new java.io.ByteArrayOutputStream();

    private static final String LOG_TAG = "TerminalSession";

    public TerminalSession(Transport transport, Integer transcriptRows, TerminalSessionClient client) {
        this.mTransport = transport;
        this.mTranscriptRows = transcriptRows;
        this.mClient = client;
    }

    /**
     * @param client The {@link TerminalSessionClient} interface implementation to allow
     *               for communication between {@link TerminalSession} and its client.
     */
    public void updateTerminalSessionClient(TerminalSessionClient client) {
        mClient = client;

        if (mEmulator != null)
            mEmulator.updateTerminalSessionClient(client);
    }

    /** Inform the remote pty of the new size and reflow or initialize the emulator. */
    public void updateSize(int columns, int rows, int cellWidthPixels, int cellHeightPixels) {
        if (mEmulator == null) {
            initializeEmulator(columns, rows, cellWidthPixels, cellHeightPixels);
        } else {
            mEmulator.resize(columns, rows, cellWidthPixels, cellHeightPixels);
        }
        mTransport.resize(columns, rows);
    }

    /** The terminal title as set through escape sequences or null if none set. */
    public String getTitle() {
        return (mEmulator == null) ? null : mEmulator.getTitle();
    }

    /**
     * Set the terminal emulator's window size and start terminal emulation.
     *
     * @param columns The number of columns in the terminal window.
     * @param rows    The number of rows in the terminal window.
     */
    public void initializeEmulator(int columns, int rows, int cellWidthPixels, int cellHeightPixels) {
        mEmulator = new TerminalEmulator(this, columns, rows, cellWidthPixels, cellHeightPixels, mTranscriptRows, mClient);
        byte[] pending;
        synchronized (mPendingBeforeEmulator) {
            pending = mPendingBeforeEmulator.toByteArray();
            mPendingBeforeEmulator.reset();
        }
        if (pending.length > 0) {
            mEmulator.append(pending, pending.length);
            notifyScreenUpdate();
        }
    }

    /** 远端输出到达（LemonGalaxy 增补，任意线程可调用）。 */
    public void appendRemoteOutput(byte[] data, int offset, int count) {
        if (count <= 0) return;
        if (Looper.myLooper() == Looper.getMainLooper()) {
            // 主线程直接处理，避免队列写满时主线程自己等自己
            if (mEmulator == null) {
                synchronized (mPendingBeforeEmulator) {
                    mPendingBeforeEmulator.write(data, offset, count);
                }
            } else {
                byte[] copy = offset == 0 ? data : java.util.Arrays.copyOfRange(data, offset, offset + count);
                mEmulator.append(copy, count);
                notifyScreenUpdate();
            }
            return;
        }
        int written = 0;
        while (written < count) {
            int chunk = Math.min(4096, count - written);
            if (!mProcessToTerminalIOQueue.write(data, offset + written, chunk)) return;
            written += chunk;
        }
        mMainThreadHandler.sendEmptyMessage(MSG_NEW_INPUT);
    }

    /** 远端会话结束（LemonGalaxy 增补，任意线程可调用）。code 为空表示未知退出码。 */
    public void onRemoteExit(Integer code) {
        mMainThreadHandler.sendMessage(mMainThreadHandler.obtainMessage(MSG_PROCESS_EXITED, code == null ? 0 : code));
    }

    /** Write data to the remote shell. */
    @Override
    public void write(byte[] data, int offset, int count) {
        if (isRunning()) mTransport.write(data, offset, count);
    }

    /** Write the Unicode code point to the terminal encoded in UTF-8. */
    public void writeCodePoint(boolean prependEscape, int codePoint) {
        if (codePoint > 1114111 || (codePoint >= 0xD800 && codePoint <= 0xDFFF)) {
            // 1114111 (= 2**16 + 1024**2 - 1) is the highest code point, [0xD800,0xDFFF] is the surrogate range.
            throw new IllegalArgumentException("Invalid code point: " + codePoint);
        }

        int bufferPosition = 0;
        if (prependEscape) mUtf8InputBuffer[bufferPosition++] = 27;

        if (codePoint <= /* 7 bits */0b1111111) {
            mUtf8InputBuffer[bufferPosition++] = (byte) codePoint;
        } else if (codePoint <= /* 11 bits */0b11111111111) {
            /* 110xxxxx leading byte with leading 5 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b11000000 | (codePoint >> 6));
            /* 10xxxxxx continuation byte with following 6 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b10000000 | (codePoint & 0b111111));
        } else if (codePoint <= /* 16 bits */0b1111111111111111) {
            /* 1110xxxx leading byte with leading 4 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b11100000 | (codePoint >> 12));
            /* 10xxxxxx continuation byte with following 6 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b10000000 | ((codePoint >> 6) & 0b111111));
            /* 10xxxxxx continuation byte with following 6 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b10000000 | (codePoint & 0b111111));
        } else { /* We have checked codePoint <= 1114111 above, so we have max 21 bits = 0b111111111111111111111 */
            /* 11110xxx leading byte with leading 3 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b11110000 | (codePoint >> 18));
            /* 10xxxxxx continuation byte with following 6 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b10000000 | ((codePoint >> 12) & 0b111111));
            /* 10xxxxxx continuation byte with following 6 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b10000000 | ((codePoint >> 6) & 0b111111));
            /* 10xxxxxx continuation byte with following 6 bits */
            mUtf8InputBuffer[bufferPosition++] = (byte) (0b10000000 | (codePoint & 0b111111));
        }
        write(mUtf8InputBuffer, 0, bufferPosition);
    }

    public TerminalEmulator getEmulator() {
        return mEmulator;
    }

    /** Notify the {@link #mClient} that the screen has changed. */
    protected void notifyScreenUpdate() {
        mClient.onTextChanged(this);
    }

    /** Reset state for terminal emulator state. */
    public void reset() {
        mEmulator.reset();
        notifyScreenUpdate();
    }

    /** Finish this terminal session by asking the remote side to close it. */
    public void finishIfRunning() {
        if (isRunning()) mTransport.close();
    }

    /** Cleanup resources when the remote session exits. */
    void cleanupResources(int exitStatus) {
        synchronized (this) {
            mRunning = false;
            mShellExitStatus = exitStatus;
        }
        mProcessToTerminalIOQueue.close();
    }

    @Override
    public void titleChanged(String oldTitle, String newTitle) {
        mClient.onTitleChanged(this);
    }

    public synchronized boolean isRunning() {
        return mRunning;
    }

    /** Only valid if not {@link #isRunning()}. */
    public synchronized int getExitStatus() {
        return mShellExitStatus;
    }

    @Override
    public void onCopyTextToClipboard(String text) {
        mClient.onCopyTextToClipboard(this, text);
    }

    @Override
    public void onPasteTextFromClipboard() {
        mClient.onPasteTextFromClipboard(this);
    }

    @Override
    public void onBell() {
        mClient.onBell(this);
    }

    @Override
    public void onColorsChanged() {
        mClient.onColorsChanged(this);
    }

    /** 远端会话没有本机进程号，恒为 0。 */
    public int getPid() {
        return 0;
    }

    /** 远端工作目录本机无法读取，恒为 null。 */
    public String getCwd() {
        return null;
    }

    @SuppressLint("HandlerLeak")
    class MainThreadHandler extends Handler {

        final byte[] mReceiveBuffer = new byte[16 * 1024];

        MainThreadHandler() {
            super(Looper.getMainLooper());
        }

        @Override
        public void handleMessage(Message msg) {
            if (mEmulator != null) {
                int bytesRead;
                while ((bytesRead = mProcessToTerminalIOQueue.read(mReceiveBuffer, false)) > 0) {
                    mEmulator.append(mReceiveBuffer, bytesRead);
                }
                notifyScreenUpdate();
            } else {
                // 仿真器尚未建立（视图还没量出尺寸）：先存起来
                int bytesRead;
                while ((bytesRead = mProcessToTerminalIOQueue.read(mReceiveBuffer, false)) > 0) {
                    synchronized (mPendingBeforeEmulator) {
                        mPendingBeforeEmulator.write(mReceiveBuffer, 0, bytesRead);
                    }
                }
            }

            if (msg.what == MSG_PROCESS_EXITED) {
                if (!isRunning()) return;
                int exitCode = (Integer) msg.obj;
                cleanupResources(exitCode);

                if (mEmulator != null) {
                    String exitDescription = "\r\n[Process completed";
                    if (exitCode > 0) {
                        exitDescription += " (code " + exitCode + ")";
                    } else if (exitCode < 0) {
                        exitDescription += " (signal " + (-exitCode) + ")";
                    }
                    exitDescription += "]";
                    byte[] bytesToWrite = exitDescription.getBytes(StandardCharsets.UTF_8);
                    mEmulator.append(bytesToWrite, bytesToWrite.length);
                    notifyScreenUpdate();
                }

                mClient.onSessionFinished(TerminalSession.this);
            }
        }

    }

}
