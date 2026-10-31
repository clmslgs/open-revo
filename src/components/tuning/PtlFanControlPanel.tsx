import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { FanCurvePoint, CustomProfileConfig } from '../../types/hardware';
import { FanCurveEditor } from './FanCurveEditor';

interface FanControlStatus {
  mode: string;
  target_rpm: [number, number];
  duty_percent?: number | null;
  cpu_temperature?: number | null;
  error?: string;
}

export const PtlFanControlPanel: React.FC<{ fan0: number; fan1: number }> = ({ fan0, fan1 }) => {
  const [control, setControl] = useState<FanControlStatus>({ mode: 'auto', target_rpm: [0, 0] });
  const [targets, setTargets] = useState<[number, number]>([4000, 4000]);
  const [curve, setCurve] = useState<FanCurvePoint[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const mounted = useRef(true);
  const refresh = async () => {
    const state = await invoke<FanControlStatus>('get_fan_control_state');
    if (mounted.current) setControl(state);
  };
  useEffect(() => {
    mounted.current = true;
    invoke<CustomProfileConfig>('get_custom_profile').then(p => {
      if (mounted.current) setCurve(p.cpu_fan_curve.map(point => ({ ...point, duty: Math.max(25, point.duty) })));
    }).catch(e => setError(String(e)));
    refresh().catch(e => setError(String(e)));
    const poll = setInterval(() => refresh().catch(e => setError(String(e))), 1500);
    return () => { mounted.current = false; clearInterval(poll); clearTimeout(timer.current); };
  }, []);
  const run = async (command: string, args?: Record<string, unknown>) => {
    clearTimeout(timer.current);
    setBusy(true);
    setError('');
    try { await invoke(command, args); await refresh(); }
    catch (e) { if (mounted.current) setError(String(e)); }
    finally { if (mounted.current) setBusy(false); }
  };
  const changeCurve = (_: 'cpu' | 'gpu', points: FanCurvePoint[]) => {
    setCurve(points);
    clearTimeout(timer.current);
    if (control.mode === 'curve') timer.current = setTimeout(() => run('set_software_fan_curve', { curve: points }), 300);
  };
  return <div className="ptl-fan-panel">
    <div className="tuning-card">
      <div className="tuning-header"><span className="tuning-header-title">双风扇控制</span></div>
      <div className="sys-row-desc">风扇 1：{fan0} RPM · 风扇 2：{fan1} RPM</div>
      <div className="ptl-fan-actions">
        <button className={`toggle-btn ${control.mode === 'auto' ? 'active-indigo' : ''}`} disabled={busy} onClick={() => run('restore_fan_auto')}>自动温控</button>
        <button className={`toggle-btn ${control.mode === 'curve' ? 'active-indigo' : ''}`} disabled={busy || curve.length < 2} onClick={() => run('set_software_fan_curve', { curve })}>启用曲线</button>
      </div>
      {[0, 1].map(index => <label className="sys-row" key={index}>
        <span className="sys-row-label">风扇 {index + 1} 目标：{targets[index]} RPM</span>
        <input aria-label={`风扇 ${index + 1} 目标转速`} type="range" min={2000} max={7500} step={100} value={targets[index]} disabled={busy}
          onChange={e => setTargets(previous => previous.map((v, i) => i === index ? Number(e.target.value) : v) as [number, number])} />
      </label>)}
      <button className={`toggle-btn ${control.mode === 'manual' ? 'active-indigo' : ''}`} disabled={busy} onClick={() => run('set_manual_fan_rpm', { cpuRpm: targets[0], gpuRpm: targets[1] })}>应用手动转速</button>
      <div className="sys-row-desc">{control.mode === 'curve' ? `曲线运行中 · CPU 封装 ${control.cpu_temperature ?? '--'}°C · 目标 PWM ${control.duty_percent ?? '--'}%` : control.mode === 'manual' ? `手动目标 ${control.target_rpm.join(' / ')} RPM` : control.mode === 'auto' ? '固件自动温控' : '正在恢复自动温控'}</div>
      <div className="sys-row-desc">曲线使用 CPU 封装温度，同步控制两只风扇；最低 PWM 25%，百分比不对应固定 RPM。温度达到 90°C、读取失败、休眠或退出时恢复自动。</div>
      {(error || control.error) && <div className="ptl-fan-error" role="alert">{error || control.error}</div>}
    </div>
    {curve.length >= 2 && <FanCurveEditor fanCurveTab="cpu" onTabChange={() => {}} cpuCurve={curve} gpuCurve={curve} onCurveChange={changeCurve}
      fanIsolated={control.mode === 'curve'} onToggleFanIsolated={() => {}} fanRampRate={false} onToggleFanRampRate={() => {}}
      fanRampSpeedMs={300} onChangeFanRampSpeed={() => {}} fanIndependentChannels={false} supportFanRampRate={false}
      fanBoostMode="none" minDuty={25} />}
  </div>;
};
