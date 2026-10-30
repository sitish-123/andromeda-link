package org.andromeda.link.ui.components

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.selection.selectable
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import org.andromeda.link.R
import org.andromeda.link.ui.theme.LinkTheme
import org.andromeda.link.ui.theme.Motion
import org.andromeda.link.ui.theme.Radius
import org.andromeda.link.ui.theme.Space
import androidx.compose.ui.geometry.Size as Box2

/** One block of a page: the page's side margins and the vertical rhythm between blocks. */
@Composable
fun Section(
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(modifier.fillMaxWidth().padding(start = Space.page, top = Space.s8, bottom = Space.s20, end = Space.page), content = content)
}

/** The bottom bar: icon over label per tab, and one soft pill behind the selected tab that slides to the next. */
@Composable
fun PillNav(
    selected: Int,
    labels: List<String>,
    icons: List<ImageVector>,
    onSelect: (Int) -> Unit,
) {
    val colors = LinkTheme.colors
    val position by animateFloatAsState(
        selected.toFloat(),
        spring(dampingRatio = 0.72f, stiffness = Spring.StiffnessMediumLow),
        label = "pill",
    )
    val bubble = colors.accent.copy(alpha = if (colors.dark) 0.18f else 0.12f)
    val rule = colors.borderPrimary
    Box(
        Modifier
            .fillMaxWidth()
            .drawBehind { drawLine(rule, Offset.Zero, Offset(size.width, 0f), 1.dp.toPx()) }
            .navigationBarsPadding()
            .padding(horizontal = Space.s12, vertical = Space.s6),
    ) {
        Canvas(Modifier.matchParentSize()) {
            val step = size.width / labels.size
            val inset = 6.dp.toPx()
            drawRoundRect(
                bubble,
                Offset(position * step + inset, 0f),
                Box2(step - inset * 2, size.height),
                CornerRadius(size.height / 2),
            )
        }
        Row(Modifier.fillMaxWidth()) {
            labels.forEachIndexed { index, label ->
                val active = index == selected
                val tint by animateColorAsState(if (active) colors.accentText else colors.textTertiary, tween(Motion.FAST), label = "tab")
                val description = stringResource(R.string.navigation_tab, label)
                Column(
                    Modifier
                        .weight(1f)
                        .height(56.dp)
                        .clip(Radius.pill)
                        .selectable(active, role = Role.Tab) { onSelect(index) }
                        .semantics { contentDescription = description },
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center,
                ) {
                    Glyph(icons[index], tint, 22.dp)
                    Label(label, LinkTheme.type.titleSmall, tint, Modifier.padding(top = Space.s2), maxLines = 1)
                }
            }
        }
    }
}

/**
 * A run of stops on a short wire, for a paged flow: stops already [done] are filled, the [current] one carries the
 * knob, and the wire is lit up to it. A stop is tappable.
 */
@Composable
fun StopTrack(
    count: Int,
    current: Int,
    done: (Int) -> Boolean,
    onSelect: (Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = LinkTheme.colors
    val position by animateFloatAsState(
        current.toFloat(),
        spring(dampingRatio = 0.62f, stiffness = Spring.StiffnessMediumLow),
        label = "stop",
    )
    val track = colors.borderPrimary
    val lit = colors.accent
    val page = colors.surfacePrimary
    Box(modifier.fillMaxWidth().height(32.dp)) {
        Canvas(Modifier.matchParentSize()) {
            val step = size.width / count
            val y = size.height / 2
            val first = step / 2
            val knob = first + position * step
            drawLine(track, Offset(first, y), Offset(first + (count - 1) * step, y), 2.dp.toPx(), StrokeCap.Round)
            drawLine(lit, Offset(first, y), Offset(knob, y), 2.5.dp.toPx(), StrokeCap.Round)
            repeat(count) { index ->
                val x = first + index * step
                drawCircle(page, 7.dp.toPx(), Offset(x, y))
                if (done(index)) {
                    drawCircle(lit, 4.5.dp.toPx(), Offset(x, y))
                } else {
                    drawCircle(track, 4.dp.toPx(), Offset(x, y), style = Stroke(1.5.dp.toPx()))
                }
            }
            drawCircle(lit, 9.dp.toPx(), Offset(knob, y), style = Stroke(2.dp.toPx()))
        }
        Row(Modifier.matchParentSize()) {
            repeat(count) { index ->
                Box(Modifier.weight(1f).fillMaxSize().selectable(index == current, role = Role.Tab) { onSelect(index) })
            }
        }
    }
}

/** A hairline rule, the only divider. */
@Composable
fun Hairline(modifier: Modifier = Modifier) {
    val color = LinkTheme.colors.borderPrimary
    Canvas(modifier.fillMaxWidth().height(1.dp)) { drawLine(color, Offset.Zero, Offset(size.width, 0f), 1.dp.toPx()) }
}

/** A short uppercase mono readout: transport, counts, states. */
@Composable
fun Readout(
    text: String,
    color: Color = LinkTheme.colors.textSecondary,
    modifier: Modifier = Modifier,
) {
    Label(text.uppercase(), LinkTheme.type.mono, color, modifier, maxLines = 1)
}
