import { describe, it, expect } from 'vitest';
import { mount } from '@vue/test-utils';
import DiffView from '../../src/components/deployment/DiffView.vue';

describe('DiffView', () => {
  it('shows "No changes" when before and after are identical', () => {
    const w = mount(DiffView, { props: { before: { 'main.tf': 'a' }, after: { 'main.tf': 'a' } } });
    expect(w.text()).toContain('No changes');
  });

  it('shows a file present only in after as added', () => {
    const w = mount(DiffView, { props: { before: {}, after: { 'new.tf': 'resource "x" {}' } } });
    const file = w.find('.diff-view__file');
    expect(file.text()).toContain('new.tf');
    expect(file.find('.diff-view__file-status--added').exists()).toBe(true);
  });

  it('shows a file present only in before as removed', () => {
    const w = mount(DiffView, { props: { before: { 'old.tf': 'resource "x" {}' }, after: {} } });
    const file = w.find('.diff-view__file');
    expect(file.text()).toContain('old.tf');
    expect(file.find('.diff-view__file-status--removed').exists()).toBe(true);
  });

  it('shows a modified file with added and removed line chunks', () => {
    const w = mount(DiffView, {
      props: {
        before: { 'main.tf': 'resource "a" {}\nresource "b" {}\n' },
        after:  { 'main.tf': 'resource "a" {}\nresource "c" {}\n' },
      },
    });
    const file = w.find('.diff-view__file');
    expect(file.find('.diff-view__file-status--modified').exists()).toBe(true);
    expect(file.find('.diff-view__chunk--removed').text()).toContain('resource "b" {}');
    expect(file.find('.diff-view__chunk--added').text()).toContain('resource "c" {}');
  });

  it('only shows files that actually changed', () => {
    const w = mount(DiffView, {
      props: {
        before: { 'unchanged.tf': 'x', 'changed.tf': 'a' },
        after:  { 'unchanged.tf': 'x', 'changed.tf': 'b' },
      },
    });
    const files = w.findAll('.diff-view__file');
    expect(files).toHaveLength(1);
    expect(files[0].text()).toContain('changed.tf');
  });

  it('sorts files alphabetically', () => {
    const w = mount(DiffView, {
      props: {
        before: {},
        after: { 'z.tf': 'z', 'a.tf': 'a' },
      },
    });
    const files = w.findAll('.diff-view__file-path');
    expect(files.map(f => f.text())).toEqual(['a.tf', 'z.tf']);
  });
});
