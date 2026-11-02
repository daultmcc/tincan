#include "helpers.h"

#include <QApplication>
#include <QIcon>
#include <QMetaObject>
#include <QQmlApplicationEngine>
#include <QUrl>

static QtMessageHandler defaultHandler = nullptr;

// Drops one known-harmless warning: on Wayland, KDE's tray integration briefly shows the tray
// menu (a QMenu with no parent window) when it's set, which Qt refuses as a parentless popup.
// The menu itself is drawn by the tray host, so nothing is lost.
static void filterMessages(QtMsgType type, const QMessageLogContext& context, const QString& message)
{
    if (type == QtWarningMsg && qstrcmp(context.category, "qt.qpa.wayland") == 0
        && message.startsWith(QLatin1String("Failed to create grabbing popup"))
        && message.contains(QLatin1String("QMenuClassWindow")))
        return;
    defaultHandler(type, context, message);
}

int run_application(rust::Str qml_url)
{
    defaultHandler = qInstallMessageHandler(filterMessages);
    static int argc = 1;
    static char arg0[] = "tincan";
    static char* argv[] = { arg0, nullptr };
    QApplication app(argc, argv);
    // TinCan keeps running in the background with no windows open.
    QApplication::setQuitOnLastWindowClosed(false);
    QApplication::setApplicationName(QStringLiteral("tincan"));
    QApplication::setApplicationDisplayName(QStringLiteral("TinCan"));
    QApplication::setDesktopFileName(QStringLiteral("tincan"));
    QApplication::setWindowIcon(QIcon(QStringLiteral(":/qt/qml/org/tincan/icons/tincan.svg")));

    QQmlApplicationEngine engine;
    engine.load(QUrl(QString::fromUtf8(qml_url.data(), static_cast<qsizetype>(qml_url.size()))));
    if (engine.rootObjects().isEmpty())
        return 1;
    return app.exec();
}

void quit_application()
{
    QMetaObject::invokeMethod(QCoreApplication::instance(), &QCoreApplication::quit, Qt::QueuedConnection);
}
