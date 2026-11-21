// xHoly Scala.js bundle for web-based deployment.
package web

import scala.scalajs.js
import scala.scalajs.js.annotation.JSExportTopLevel

/** Browser entry point: minimal UI shell that reports emulator status. */
@JSExportTopLevel("xHolyBoot")
def boot(): js.Dynamic = {
  js.Dynamic.global.document
    .getElementById("xholy-status")
    .innerHTML = "xHoly web runtime: scaffold (see docs/whitepaper.md §7)"
}
