# 문법 요약

매뉴얼을 한 장씩 넘겨가면서 모든 타입을 배웠으니, 생성과 소멸 문법을 한 자리에 모아볼 수 있다면 편리할 것이다.

이번 장은 단순 문법 요약으로, 앞 장의 자세한 설명을 대체하는 용도가 아니다. 첫 번째 표는 지금까지 확인한 식 문법을 요약한 것이다.

두 번째 표도 위와 동일하지만, 프로세스 문법을 기준으로 요약한 것이다. 아직 프로세스 문법을 배우지 않았다면 일단 무시한 뒤 나중에 확인하면 된다.

## 식 문법

<table>

<tr>
<td><strong>타입</strong></td>
<td><strong>생성</strong></td>
<td><strong>소멸</strong></td>
</tr>

<tr>
<td><pre><code class="language-par">// 단위
type Unit = !</code></pre></td>
<td><pre><code class="language-par">let value = !</code></pre></td>
<td><pre><code class="language-par">let ! = value</code></pre></td>
</tr>

<tr/>

<tr>
<td><pre><code class="language-par">// 분기
type Either = either {
  .left String,
  .right Int,
}</code></pre></td>
<td><pre><code class="language-par">let value: Either = .left "Hello!"</code></pre></td>
<td><pre><code class="language-par">let result = value.case {
  .left str => str,
  .right num => `#{num}`,
}</code></pre></td>
</tr>

<tr/>

<tr>
<td><pre><code class="language-par">// 순서쌍
type Pair = (String) Int</code></pre></td>
<td><pre><code class="language-par">let value = ("Hello!") 42</code></pre></td>
<td><pre><code class="language-par">let (str) num = value</code></pre></td>
</tr>

<tr/>

<tr>
<td><pre><code class="language-par">// 함수
type Function = [Int] String</code></pre></td>
<td><pre><code class="language-par">let value = [num: Int] `#{num}`</code></pre></td>
<td><pre><code class="language-par">let str = value(42)</code></pre></td>
</tr>

<tr/>

<tr>
<td><pre><code class="language-par">// 선택
type Choice = choice {
  .left => String,
  .right => Int,
}</code></pre></td>
<td><pre><code class="language-par">let value: Choice = case {
  .left => "Hello!",
  .right => 42,
}</code></pre></td>
<td><pre><code class="language-par">let num = value.right</code></pre></td>
</tr>

<tr/>

<tr>
<td><pre><code class="language-par">// 후속문
type Continuation = ?</code></pre></td>
<td><em>식 문법 없음</em></td>
<td><em>식 문법 없음</em></td>
</tr>

</table>

## 프로세스 문법

<table>

<tr>
<td><strong>타입</strong></td>
<td><strong>생성</strong></td>
<td><strong>소멸</strong></td>
</tr>

<tr>
<td><pre><code class="language-par">// 단위
type Unit = !</code></pre></td>
<td><pre><code class="language-par">let value = chan c {
  c!
}</code></pre></td>
<td><pre><code class="language-par">value?</code></pre></td>
</tr>

<tr/>

<tr>
<td><pre><code class="language-par">// 분기
type Either = either {
  .left String,
  .right Int,
}</code></pre></td>
<td><pre><code class="language-par">let value: Either = chan c {
  c.left
  c &lt;&gt; "Hello!"
}</code></pre></td>
<td><pre><code class="language-par">value.case {
  .left => {
    let result = value
  }
  .right => {
    let result = `#{value}`
  }
}
// `result`가 범위 안에 들어옴
</code></pre></td>
</tr>

<tr/>

<tr>
<td><pre><code class="language-par">// 순서쌍
type Pair = (String) Int</code></pre></td>
<td><pre><code class="language-par">let value = chan c {
  c("Hello!")
  c &lt;&gt; 42
}</code></pre></td>
<td><pre><code class="language-par">value[str]
let num = value</code></pre></td>
</tr>

<tr/>

<tr>
<td><pre><code class="language-par">// 함수
type Function = [Int] String</code></pre></td>
<td><pre><code class="language-par">let value = chan c {
  c[num: Int]
  c &lt;&gt; `#{num}`
}</code></pre></td>
<td><pre><code class="language-par">value(42)
let result = value</code></pre></td>
</tr>

<tr/>

<tr>
<td><pre><code class="language-par">// 선택
type Choice = choice {
  .left => String,
  .right => Int,
}</code></pre></td>
<td><pre><code class="language-par">let value = chan c {
  c.case {
    .left  => { c &lt;&gt; "Hello!" }
    .right => { c &lt;&gt; 42 }
  }
}</code></pre></td>
<td><pre><code class="language-par">value.right
let num = value</code></pre></td>
</tr>

<tr/>

<tr>
<td><pre><code class="language-par">// 후속문
type Continuation = ?</code></pre></td>
<td><pre><code class="language-par">let outer: ! = chan break {
  let value: ? = chan c {
    c?     // 생성
    break!
  }
  value!   // 소멸
}</code></pre></td>
<td><em>좌측 참조</em></td>
</tr>

</table>
