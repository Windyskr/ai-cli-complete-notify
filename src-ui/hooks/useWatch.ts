import { useState, useCallback, useRef } from 'react';
import { sidecar, spawnSidecar } from '@/lib/sidecar';
import { dispatchNativeNotificationLine } from '@/lib/native-notification';
import { stopNativeWatch } from '@/lib/window';
import type { Child } from '@tauri-apps/plugin-shell';

export function useWatch() {
  const [running, setRunning] = useState(false);
  const [logs, setLogs] = useState<string[]>([]);
  const childRef = useRef<Child | null>(null);
  const stdoutBufferRef = useRef('');

  const appendLog = useCallback((line: string) => {
    const ts = new Date().toLocaleTimeString('en-GB', { hour12: false });
    const stamped = `[${ts}] ${line}`;
    setLogs((prev) => {
      const next = [...prev, stamped];
      return next.length > 500 ? next.slice(-500) : next;
    });
  }, []);

  const handleStdout = useCallback((chunk: string) => {
    const lines = `${stdoutBufferRef.current}${String(chunk || '')}`.split(/\r?\n/);
    stdoutBufferRef.current = lines.pop() || '';

    for (const line of lines) {
      if (!line.trim()) continue;
      if (!dispatchNativeNotificationLine(line)) appendLog(line);
    }
  }, [appendLog]);

  const start = useCallback(
    async (opts: {
      sources?: string;
      intervalMs?: number;
      geminiQuietMs?: number;
      claudeQuietMs?: number;
    }) => {
      if (childRef.current) return;
      const args = [
        'watch',
        '--sources', opts.sources || 'all',
        '--interval-ms', String(opts.intervalMs ?? 1000),
        '--gemini-quiet-ms', String(opts.geminiQuietMs ?? 3000),
        '--claude-quiet-ms', String(opts.claudeQuietMs ?? 60000),
      ];
      try {
        // Lightweight mode may leave a Rust-owned watch running; stop it first
        // so we do not end up with two watchers after the UI is restored.
        await stopNativeWatch().catch(() => undefined);
        stdoutBufferRef.current = '';
        const child = await spawnSidecar(
          args,
          handleStdout,
          (line) => appendLog(`[stderr] ${line}`),
        );
        childRef.current = child;
        setRunning(true);
        appendLog('[watch] started');
      } catch (e) {
        appendLog(`[watch] failed to start: ${e}`);
      }
    },
    [appendLog, handleStdout],
  );

  const stop = useCallback(async () => {
    if (childRef.current) {
      try {
        await childRef.current.kill();
      } catch {
        // ignore
      }
      childRef.current = null;
    }
    setRunning(false);
    appendLog('[watch] stopped');
  }, [appendLog]);

  const clearLogs = useCallback(() => setLogs([]), []);

  return { running, logs, start, stop, clearLogs, appendLog };
}
