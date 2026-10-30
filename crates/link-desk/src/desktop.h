#pragma once

#include <QtCore/QMimeData>
#include <QtCore/QStandardPaths>
#include <QtCore/QStringList>
#include <QtCore/QUrl>
#include <QtGui/QClipboard>
#include <QtGui/QDesktopServices>
#include <QtGui/QGuiApplication>

inline void setClipboardText(const QString& text)
{
  QGuiApplication::clipboard()->setText(text);
}

inline void setClipboardFiles(const QStringList& paths)
{
  QList<QUrl> urls;
  for (const QString& path : paths) {
    urls.append(QUrl::fromLocalFile(path));
  }
  auto* data = new QMimeData;
  data->setUrls(urls);
  QGuiApplication::clipboard()->setMimeData(data);
}

inline bool openPath(const QString& path)
{
  return QDesktopServices::openUrl(QUrl::fromLocalFile(path));
}

inline QString downloadsFolder()
{
  return QStandardPaths::writableLocation(QStandardPaths::DownloadLocation);
}

inline QString picturesFolder()
{
  return QStandardPaths::writableLocation(QStandardPaths::PicturesLocation);
}

inline void quitApp()
{
  QCoreApplication::quit();
}

inline void keepRunningWithoutWindows()
{
  QGuiApplication::setQuitOnLastWindowClosed(false);
}
