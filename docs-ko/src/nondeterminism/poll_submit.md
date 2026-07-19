# 폴링과 제출

Par에서는 독립적인 클라이언트 행위자들이 중앙 서버 행위자와 통신하는 서버-클라이언트 비결정성 문제를 어떻게 해결할까?

Par에서는 **`poll`/`submit` 제어 구문** 하나로 문제를 해결할 수 있다.

별도로 도입되는 타입은 없다. 서버와 클라이언트 사이의 통신은 기존의 타입만으로도 충분히 기술할 수 있다.

## 발상: 클라이언트 풀

문법을 알아보기 전에 먼저 기초 개념을 잡아 보자. `poll`/`submit` 구문은 배후에서 *클라이언트 풀*을 운영하는 형태로 구현되어 있다. 풀이란 몇 개의 개체를 모아놓고 대기시키는 공간이라고 생각하면 된다. 맥락에 따라 *행위자*나 *클라이언트*라고 부를 수도 있지만, 어떤 값이든 풀에 넣을 수 있다.

같은 풀 안에 있는 클라이언트는 모두 같은 타입을 가진다.

풀 안의 클라이언트는 어떤 **동작을 수행할 수 있을 때까지 대기**한다.
- [순서쌍](../types/pair.md)의 경우에는 송신 명령에 해당한다.
- [함수](../types/function.md)의 경우에는 수신 명령에 해당한다.
- [분기](../types/either.md)의 경우에는 선택 명령에 해당한다.
- [선택](../types/choice.md)의 경우에는 분기 명령에 해당한다.

준비를 마친 클라이언트는 **풀에서 폴링**되어 폴러인 서버 프로세스에 전달된다. 서버가 이 클라이언트를 처리하고 나면 0개 이상의 신규 클라이언트를 **풀에 다시 제출**한 뒤 폴링을 계속한다.

## `poll` / `submit`

`poll`/`submit`은 대부분의 경우 식 문법을 사용하는 것이 가장 자연스러우므로 이쪽을 먼저 살펴 보도록 하자.

`poll`식의 전반적인 문법은 다음과 같다.

```par
poll(<client-value>, ...) {
  <client-variable> => <result-for-non-empty-pool>

  else => <result-after-empty-pool>
}
```

`poll` 키워드 다음에는 **초기 클라이언트의 목록**을 소괄호로 감싸서 작성한다. 초기에는 최소 하나의 클라이언트가 있어야 한다.

그 뒤에는 중괄호 안에 정확히 두 개의 조건지를 작성한다.
- **활성 조건지**: 어떤 클라이언트가 준비 상태가 되면 이 조건지가 호출되어 해당 클라이언트가 `<client-variable>`에 대입된다. 이 조건지에서는 **반드시 `submit`을 호출해야 한다**.
- **`else` 조건지**: 풀이 비어 있는 상태가 되면 `else` 조건지로 진입하여 최종 결과를 생성한다.

**활성 조건지 안에서는 `submit`을 정확히 한 번 호출해야 한다.**

```par
submit(<client-value>, ...)
```

`poll`과는 달리 신규 클라이언트를 제출하지 않는 것도 가능하다.

```par
submit()
```

`submit`이 호출되면 신규 클라이언트를 풀에 삽입하고 폴링을 재개한다.

`submit`식은 해당 `submit`에 대응하는 `poll` 전체의 결과값을 반환한다. [`.begin`/`.loop`](../types/recursive.md#소멸)와 거의 같은 동작을 하며, 지역 변수도 동일하게 유지된다. 즉, `poll`에서 사용한 지역 변수는 `submit`을 호출할 때 다음 `poll`로 전달된다.

**`poll`에는 두 가지 중요한 제한이 있다.**
- 클라이언트는 반드시 [재귀](../types/recursive.md) 타입이어야 한다.
- `submit`으로는 활성 조건지의 클라이언트에서 **파생된** 값만 제출할 수 있다. 이는 전체성을 보장하기 위한 제한으로, 같은 클라이언트를 그대로 풀에 다시 제출하는 식으로 무한루프를 일으키는 것은 불가능하다.

예제를 확인하면서 기능을 이해해 보자.

## 간단한 예제

`.begin`/`.loop`로 구현할 수 있는 함수를 `poll`/`submit`으로 재구현하면서 감을 잡는 것이 가장 쉬운 방법이다.

> `poll`/`submit`은 `.begin`/`.loop`의 대체재가 아니다. 한쪽의 기능을 다른 쪽으로 완전히 대체할 수는 없으며, 일부 제한된 경우에만 가능하다.

**리스트의 합을 구해 보자!**

```par
module Main

import {
  @core/Int
  @core/List
}

dec PollSum : [List<Int>] Int
def PollSum = [nums] poll(nums) {
  list => list.case {
    .end! => submit(),
    .item(x) xs => x + {submit(xs)},
  }
  else => 0,
}
```

한 부분씩 살펴 보자.

- `poll(nums)`
  
  하나의 클라이언트 `nums`로 이루어진 풀을 생성한다.
- `list =>`
  
  클라이언트가 활성화되어 새로운 변수 `list`에 대입된다.
- `list.case`
  
  특별할 것 없는 [`.case`](../types/either.md)식이다.
- `.end! => submit()`
  
  리스트가 비어 있으면 빈 `submit()`을 호출한다. 이 식은 남은 풀로 다음 `poll`을 실행한 결과를 반환하며, 이 경우에는 `else` 조건지에 해당한다.
- `.item(x) xs => x + {submit(xs)},`
  
  리스트에 값이 있을 경우, 리스트의 나머지 부분을 풀에 다시 `submit(xs)`하고 다음 `poll`의 결과를 리스트의 첫 원소에 더해 반환한다.

`.begin`/`.loop`를 사용해 구현한 것과 큰 차이가 없음을 알 수 있다.

```par
dec Sum : [List<Int>] Int
def Sum = [nums] nums.begin.case {
  .end! => 0,
  .item(x) xs => x + xs.loop,
}
```

`xs.loop` 대신 `submit(xs)`를 하는 것 이외에도 중요한 차이가 또 하나 있다.

`.begin`/`.loop`에서는 `.end!` 분지에 도달하면 재귀가 끝났다는 것을 알 수 있다. 하지만 풀을 사용할 때는 한 클라이언트가 끝났다고 해서 풀 전체가 바로 종료되는 것이 아니다. `.end!` 분지에서도 `submit`해야 하는 이유가 바로 이것이다!

물론 `PollSum` 함수를 확장하여 두 리스트의 합을 구하도록 하는 것도 매우 간단하다!

```par
dec PollSumTwo : [List<Int>, List<Int>] Int
def PollSumTwo = [nums1, nums2] poll(nums1, nums2) {
  list => list.case {
    .end! => submit(),
    .item(x) xs => x + {submit(xs)},
  }
  else => 0,
}
```

아래의 단 한 줄을...

```par
def PollSum = [nums] poll(nums) {
```

다음과 같이 수정하기만 하면 된다.

```par
def PollSumTwo = [nums1, nums2] poll(nums1, nums2) {
```

코드의 다른 부분은 전혀 건드리지 않았다.

이제 `.end!` 분지의 빈 `submit()`이 이해가 될 것이다! 리스트 둘 중 하나가 끝났다고 해서 더 이상 클라이언트가 없다고 생각하면 안 된다. 아직 다른 리스트가 남아 있다!

**이번에는 두 리스트를 하나로 합쳐 보자.**

```par
dec MergeTwoLists : [<a> List<a>, List<a>] List<a>
def MergeTwoLists = [<a> left, right] poll(left, right) {
  list => list.case {
    .end! => submit(),
    .item(x) xs => .item(x) submit(xs),
  }
  else => .end!,
}
```

같은 원리이지만, 원소를 더하는 대신 리스트를 결과로 생성하고 있다.

> 반환되는 리스트에서 원소의 순서가 아무렇게나 섞이는 것이 아님을 염두에 두어야 한다! 리스트의 나머지 부분은 직전 원소를 리스트에 삽입하고 나서야 풀에 제출된다. 즉, 예를 들어서...
>
> ```par
> MergeTwoLists(*(1, 2, 3), *(4, 5, 6))
> ```
>
> 위 식에서 다음 결과가 나오는 것은 모두 가능하지만...
> - `*(1, 4, 2, 5, 3, 6)`
> - `*(1, 2, 3, 4, 5, 6)`
> - `*(4, 5, 6, 1, 2, 3)`
>
> `*(6, 5, 4, 3, 2, 1)`은 불가능하다!

**트리를 비결정론적으로 리스트로 만드는 것은 어떨까?**

```par
type Tree<a> = recursive either {
  .leaf a,
  .node(self) self,
}
```

누워서 떡 먹기다.

```par
dec TreeToList : [<a> Tree<a>] List<a>
def TreeToList = [<a> tree] poll(tree) {
  tree => tree.case {
    .leaf x => .item(x) submit(),
    .node(l) r => submit(l, r),
  }
  else => .end!,
}
```

반환되는 리스트의 순서는 단말 노드가 풀에 들어온 순서에만 의존한다.

여기에서는 처음으로 여러 개의 클라이언트를 동시에 `submit`하는 것 역시 확인할 수 있다.

```par
    .node(l) r => submit(l, r),
```

`.loop`는 같은 `.begin` 안에서 몇 번이든 사용할 수 있지만, 한 번에 정확히 하나의 값에만 적용할 수 있다. 한편 `submit`은 같은 `poll` 안에서 정확히 한 번만 사용할 수 있지만, 일단 사용할 때는 몇 개의 값이든 제출할 수 있다.

## 프로세스 문법

이번 징에서는 지금까지 모든 예제에서 식 문법을 사용했지만, 프로세스 문법에서도 `poll`/`submit`을 동일하게 사용할 수 있다.

```par
// 프로세스 안에서
poll(client1, client2) {
  client => {
    // ... 클라이언트 처리
    submit(client)
  }

  else => {
    // ... 나머지 프로세스를 처리
  }
}
```

여기서 `poll`문과 `submit`문은 제어 흐름 구문이므로 값을 '반환'하지 않는다. `submit`문은 `poll`로 되돌아가면서 클라이언트를 풀에 되돌리는 것 외에는 특별한 기능을 하지 않는다.
