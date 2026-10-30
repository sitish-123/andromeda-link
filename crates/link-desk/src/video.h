#pragma once

#include <cstdint>
#include <cstring>

#include <QtMultimedia/QVideoFrame>
#include <QtMultimedia/QVideoFrameFormat>
#include <QtMultimedia/QVideoSink>

#include "rust/cxx.h"

// Shows one NV12 picture: `luma` rows of `stride` bytes, then the interleaved chroma at half height.
inline void showNv12(QVideoSink* sink, std::int32_t width, std::int32_t height, std::int32_t stride,
                     rust::Slice<const std::uint8_t> data)
{
  if (sink == nullptr) {
    return;
  }
  QVideoFrame frame(QVideoFrameFormat(QSize(width, height), QVideoFrameFormat::Format_NV12));
  if (!frame.map(QVideoFrame::WriteOnly)) {
    return;
  }
  const std::uint8_t* src = data.data();
  for (int plane = 0; plane < 2; ++plane) {
    const int rows = plane == 0 ? height : height / 2;
    for (int row = 0; row < rows; ++row) {
      std::memcpy(frame.bits(plane) + row * frame.bytesPerLine(plane), src + row * stride, width);
    }
    src += static_cast<std::size_t>(stride) * rows;
  }
  frame.unmap();
  sink->setVideoFrame(frame);
}
