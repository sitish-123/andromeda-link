#pragma once

#include <QtGui/QClipboard>
#include <QtGui/QGuiApplication>

inline QString clipboardText()
{
  return QGuiApplication::clipboard()->text();
}
