# 소멸에 의한 생성

지금까지 채널을 사용해 프로세스에서 조기 반환을 하면서 `chan`식을 직접 사용해 보았다. 하지만 이는 빙산의 일각이다!

`chan`식에서 얻는 채널은 단순한 반환 핸들이 아닌, **결과값의 사용자에게 직접 연결되는 통로**이다. 즉, 이 채널과 점진적으로 상호작용하여 단계적으로 값을 생성하는 것도 가능하다.

이번 장의 제목인 '*소멸에 의한 생성*'이 잘 들어맞는다고 할 수 있다! 이번 장에서 배울 내용은 수학계에서 유명한 증명 기법인 *귀류법*과 일맥상통하고, 위력 역시 뒤처지지 않는다.

## 쌍대성의 이론

Par의 모든 타입에는 통신의 반대편 역할을 하는 **쌍대**가 있다.

타입 연산자 `dual <type>`으로 주어진 타입을 그 쌍대로 변환할 수 있다. 구조적으로 정의하면 다음과 같다.

<table>

<tr/>
<tr>
<td><code class="language-par">dual !</code></td>
<td><strong>＝</strong></td>
<td><code class="language-par">&#63;</code></td>
</tr>

<tr/>
<tr>
<td><code class="language-par">dual &#63;</code></td>
<td><strong>＝</strong></td>
<td><code class="language-par">!</code></td>
</tr>

<tr/>
<tr>
<td><code class="language-par">dual (A) B</code></td>
<td><strong>＝</strong></td>
<td><code class="language-par">[A] dual B</code></td>
</tr>

<tr/>
<tr>
<td><code class="language-par">dual [A] B</code></td>
<td><strong>＝</strong></td>
<td><code class="language-par">(A) dual B</code></td>
</tr>

<tr/>
<tr>
<td><pre><code class="language-par">dual either {
  .left A,
  .right B,
}</code></pre></td>
<td><strong>＝</strong></td>
<td><pre><code class="language-par">choice {
  .left => dual A,
  .right => dual B,
}</code></pre></td>
</tr>

<tr/>
<tr>
<td><pre><code class="language-par">dual choice {
  .left => A,
  .right => B,
}</code></pre></td>
<td><strong>＝</strong></td>
<td><pre><code class="language-par">either {
  .left dual A,
  .right dual B,
}</code></pre></td>
</tr>

<tr/>
<tr>
<td><code class="language-par">dual recursive F&lt;self&gt;</code></td>
<td><strong>＝</strong></td>
<td><code class="language-par">iterative dual F&lt;dual self&gt;</code></td>
</tr>

<tr/>
<tr>
<td><code class="language-par">dual iterative F&lt;self&gt;</code></td>
<td><strong>＝</strong></td>
<td><code class="language-par">recursive dual F&lt;dual self&gt;</code></td>
</tr>

<tr/>
<tr>
<td><code class="language-par">dual [type a] F&lt;a&gt;</code></td>
<td><strong>＝</strong></td>
<td><code class="language-par">(type a) dual F&lt;a&gt;</code></td>
</tr>

<tr/>
<tr>
<td><code class="language-par">dual (type a) F&lt;a&gt;</code></td>
<td><strong>＝</strong></td>
<td><code class="language-par">[type a] dual F&lt;a&gt;</code></td>
</tr>

</table>

> 재귀·제네릭 타입의 `F<...>`를 보고 겁먹지 않아도 된다. `recursive`/`iterative`를 `iterative`/`recursive`로 바꾼 뒤 본문의 쌍대를 마저 구한다는 것을 엄밀하게 표현한 것뿐이다.

위의 표를 보고 다음을 알 수 있다.
- [단위](../types/unit.md)와 [후속문](../types/continuation.md)은 서로 쌍대 관계이다.
- [함수](../types/function.md)와 [순서쌍](../types/pair.md)은 서로 쌍대 관계이다.
- [분기](../types/either.md)와 [선택](../types/choice.md)은 서로 쌍대 관계이다.
- [재귀](../types/recursive.md)와 [반복](../types/iterative.md)은 서로 쌍대 관계이다.
- [전칭](../types/forall.md)과 [존재](../types/exists.md)는 서로 쌍대 관계이다.
- **쌍대 관계는 항상 양방향이다!**

마지막 관찰이 중요하다. 실제로 어떤 타입 `A`에 대해서도 `dual dual A`가 `A`와 같음을 보일 수 있다.

## 쌍대성의 실제

익숙한 이 타입을 살펴 보자.

```par
type List<a> = recursive either {
  .end!,
  .item(a) self,
}
```

이 타입의 쌍대는 무엇일까? 위의 규칙을 적용하면 다음과 같다.

```par
iterative choice {
  .end => ?,
  .item(a) => self,
}
```

`List<a>`가 값을 주다가 종료한다면, 쌍대 타입은 *우리*가 (`.item` 분지로) 값을 주다가 종료해야 한다. 리스트의 사용자가 누구가 되었든, 쌍대 타입을 사용하면 그 사용자와 통신할 수 있다.

`.end` 분지에 `?` 타입이 있는데, 이는 **후속문** 타입이다. 이 타입은 식 문법이 없어서 다룰 수 없었는데, 이제 프로세스에서 이 타입을 다루는 방법을 배울 수 있게 되었다!

`chan`식의 블록 안에서 얻는 채널의 타입은 식의 최종 결과의 **쌍대 타입**이라는 점을 기억하면서 리스트를 점진적으로 생성해 보자.

```par
module Main

import {
  @core/Int
  @core/List
}

def SmallList: List<Int> = chan yield {
  yield.item(1)
  yield.item(2)
  yield.item(3)
  yield.end
  yield!
}

// def SmallList = *(1, 2, 3)
```

마지막 줄을 제외하면 일반적인 [반복](../types/iterative.md)
[선택](../types/choice.md) 값을 다루는 것과 차이가 없음을 알 수 있다.

`.end` 분지를 선택하면 `yield` 채널이 후속문 타입 `?`가 된다. 이 시점에서 프로토콜은 종료되었으므로, 유일하게 **탈출 명령**만 사용할 수 있다. 마지막 줄에 `!`로 적으면 된다.

```par
  yield!  // 프로세스에서 탈출
```

[연결](./chan_expression.md#-명령을-사용한-연결) 명령과 같이 이 명령 역시 프로세스의 마지막 명령이어야 한다. 사실 **연결과 탈출 외에 프로세스를 종료하는 방법은 없다.**

## 실제 용례: 리스트의 리스트 평탄화

이제 이 스타일로 의미 있는 코드를 구현해 보자. 여기서는 중첩된 리스트를 한 겹으로 평탄화하는 함수를 작성할 것이다.

```par
dec Flatten : [type a, List<List<a>>] List<a>
```

이 함수는 제네릭으로, [전칭](../types/forall.md) 타입을 사용해 다형성을 만족한다.

이 함수 전체의 명령형 스타일 구현은 다음과 같다.

```par
def Flatten = [type a, lists] chan yield {
  lists.begin@outer.case {
    .end! => {
      yield.end!
    }

    .item(list) => {
      list.begin@inner.case {
        .end! => {}
        .item(value) => {
          yield.item(value)
          list.loop@inner
        }
      }
      lists.loop@outer
    }
  }
}
```

지금까지 배운 여러 가지 개념을 위의 코드에서 모두 사용하고 있다.

- `.begin@outer`와 `.loop@outer`로 바깥쪽 리스트를 순회한다.
- `.end!` 분지일 경우, `yield.end!`로 종료한다.
- `.item(list)` 분지일 경우, 안쪽 리스트를 순회한다.
  - 이때 리스트의 나머지 부분을 수동으로 재대입하지 않아도 원래 변수인 `lists`를 통해 통신이 계속된다.
  - 헬퍼 함수를 사용하지 않고도 중첩된 `.begin`/`.loop`로 순회하는 것이 가능하다.
- 안쪽 루프에서는...
  - `.end!` 분지일 경우, 그 리스트를 모두 사용한 것이므로 바깥쪽 루프를 속행한다.
  - `.item(value)` 분지일 경우, `yield.item(value)`로 그 값을 사용자에게 전달한 뒤 계속 순회한다. 이때도 리스트의 나머지 부분을 재대입하지 않아도 된다.

`chan`식과 쌍대성이 결합하면 놀라운 표현력을 자랑한다.\
제너레이터 스타일로 리스트를 생성하는 것은 수많은 용례 중 하나일 뿐이다.\
자원을 모아서 값을 생성하는 대신 *직접 값이 되어야 하는* 상황이라면, `chan`식과 쌍대성이 분명 도움이 될 것이다!
