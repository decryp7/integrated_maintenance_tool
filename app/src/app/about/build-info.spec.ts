import { TestBed } from '@angular/core/testing';
import { BUILD_INFO, parseBuildInfo, UNKNOWN_BUILD } from './build-info';

describe('build info', () => {
  it('defaults to unknown when the build defined nothing (dev builds, tests)', () => {
    expect(TestBed.inject(BUILD_INFO)).toEqual(UNKNOWN_BUILD);
  });

  it('keeps a well-formed commit and date', () => {
    expect(parseBuildInfo({ commit: 'abc1234', date: '2026-10-17' })).toEqual({ commit: 'abc1234', date: '2026-10-17' });
  });

  it('turns anything malformed into unknown', () => {
    expect(parseBuildInfo(undefined)).toEqual(UNKNOWN_BUILD);
    expect(parseBuildInfo('abc1234')).toEqual(UNKNOWN_BUILD);
    expect(parseBuildInfo({ commit: '<script>', date: 20261017 })).toEqual(UNKNOWN_BUILD);
    expect(parseBuildInfo({ commit: 'abc1234', date: 'yesterday' })).toEqual({ commit: 'abc1234', date: 'unknown' });
  });
});
