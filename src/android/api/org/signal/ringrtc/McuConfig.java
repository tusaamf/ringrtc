/*
 * Copyright 2025 Signal Messenger, LLC
 * SPDX-License-Identifier: AGPL-3.0-only
 */

package org.signal.ringrtc;

import java.util.Objects;

public class McuConfig {
  public int baudrate;
  public long sleepUs;
  public int retryQuota;
  public String spidevPath;
  public boolean enable;
  public boolean debug;

  public McuConfig() {
    this.baudrate = 10;
    this.sleepUs = 1000;
    this.retryQuota = 10;
    this.spidevPath = "/dev/spidev0.0";
    this.enable = false;
    this.debug = false;
  }

  @Override
  public String toString() {
    return "McuConfig{" +
           "baudrate=" + baudrate +
           ", sleepUs=" + sleepUs +
           ", retryQuota=" + retryQuota +
           ", spidevPath=" + spidevPath +
           ", enable=" + enable +
           ", debug=" + debug +
           "}";
  }

  @Override
  public boolean equals(Object o) {
    if (this == o) return true;
    if (o == null || getClass() != o.getClass()) return false;
    McuConfig that = (McuConfig) o;
    return baudrate == that.baudrate &&
           sleepUs == that.sleepUs &&
           retryQuota == that.retryQuota &&
           spidevPath == that.spidevPath &&
           enable == that.enable &&
           debug == that.debug;
  }

  @Override
  public int hashCode() {
    return Objects.hash(baudrate, sleepUs, retryQuota, spidevPath, enable, debug);
  }
}
