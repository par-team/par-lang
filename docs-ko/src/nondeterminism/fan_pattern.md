# 팬 패턴

이전 장에서는 두 리스트를 원소가 들어오는 타이밍에 따라 하나로 합쳐 보고...

```par
module Main

import @core/List

dec MergeTwoLists : [<a> List<a>, List<a>] List<a>
def MergeTwoLists = [<a> left, right] poll(left, right) {
  list => list.case {
    .end! => submit(),
    .item(x) xs => .item(x) submit(xs),
  }
  else => .end!,
}
```

트리 역시 리스트로 변환해 보았다.

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

이 방식으로 여러 개의 느린 정보원을 하나의 스트림으로 합치면 데이터가 생성되자마자 즉시 반응하도록 할 수 있다.

**그런데 리스트의 리스트는 어떻게 합쳐야 할까?**

```par
dec MergeLists : [<a> List<List<a>>] List<a>
def MergeLists = ???
```

원소가 생성되는 순서를 따라 합치고 싶다면, **`poll`/`submit`만으로는 문제에 봉착하게 된다.**

같은 풀 안의 모든 클라이언트는 같은 타입이어야 한다. 그런데 풀에 `List<List<a>>`를 삽입하면 그 다음에도 `List<List<a>>` 타입의 클라이언트를 얻는다. 정작 폴링해야 할 안쪽 리스트에는 접근할 수 없다!

> 이 문제는 `poll`/`submit`과 `.begin`/`.loop`를 조합해서 해결할 수는 있다.
>
> ```par
> dec MergeLists : [<a> List<List<a>>] List<a>
> def MergeLists = [<a> lists] lists.begin.case {
>   .end! => .end!,
>   .item(list) lists => poll(list, lists.loop) {
>     list => list.case {
>       .end! => submit(),
>       .item(x) xs => .item(x) submit(xs),
>     }
>     else => .end!,
>   }
> }
> ```
>
> 위의 코드에서는 풀에 단순 리스트를 삽입하고 있다. 하지만 모든 리스트마다 풀이 하나씩 생성되기 때문에, 풀의 (`poll`을 통해 연결된) 연결 리스트를 만들게 된다. 문제는 해결됐지만, 효율적이라고는 하기 어렵다.

문제를 **효율적으로 해결**하는 방법으로는 `List<List<a>>`를 폴링에 적합한 자료구조로 변환하는 것이 있다! 이 패턴을 ***팬(fan)* 패턴**이라고 부른다.

## 동질적 트리, 팬

`poll`이 `List<a>`와 `Tree<a>`에는 적합한데 `List<List<a>>`에는 부적합한 이유는 무엇일까? `List<a>`와 `Tree<a>`는 모두 <em>동질적(homogeneous)</em>인 타입이라는 공통점이 있기 때문이다.

```par
type List<a> = recursive either {
  .end!,
  .item(x) self,
}

type Tree<a> = recursive either {
  .leaf a,
  .node(self) self,
}
```

위 타입을 잘 보면, **모든 `self`가 같은 모양을 가리키고 있다.** 리스트의 리스트에 대해서는 이 성질이 성립하지 않는다! `List<List<a>>`의 바깥 노드는 다음과 같다.

```par
recursive either {
  .end!,
  .item(List<a>) self,
}
```

한편 안쪽 노드(`List<a>`)는 다음과 같다.

```par
recursive either {
  .end!,
  .item(a) self,
}
```

위의 차이 때문에 `poll`을 깔끔하게 사용하지 못한 것이다.

## *팬* 변환

`List<List<a>>`를 적합한 자료구조로 변환해야 문제를 해결할 수 있다. 다음 타입을 사용하면 된다.

```par
type ListFan<a> = recursive either {
  .end!,
  .spawn(self) self,
  .item(a) self,
}
```

팬 타입을 설계하는 데는 따로 공식이 없고, 용례를 보고 판단해야 한다. 하지만 아래의 두 분지는 자주 볼 수 있을 것이다.

```par
  .end!,
  .spawn(self) self,
```

위의 두 분지로써 **자료구조를 동적으로 여러 행위자로 확장시키는 것**이 가능하며, 나머지 분지는 단일 행위자, 여기서는 리스트 하나의 동작을 구현한다.

리스트의 팬 변환은 간단히 구현할 수 있다.

```par
dec ListFan : [<a> List<List<a>>] ListFan<a>
def ListFan = [<a> lists] lists.begin.case {
  .end! => .end!,
  .item(list) lists => .spawn(
    list.begin.case {
      .end! => .end!,
      .item(x) xs => .item(x) xs.loop,
    }
  ) lists.loop,
}
```

변환은 두 겹으로 중첩된 재귀로 이루어진다.
- 바깥 재귀에서 각 리스트마다 `.spawn` 노드를 생성한다.
- 안쪽 재귀에서 리스트의 각 원소마다 `.item` 노드를 생성한다.
- 반환된 팬 자료구조에서는 `.end` 노드가 두 가지 역할을 한다.
  1. 더 이상 리스트가 없음을 나타낸다.
  2. 단일 리스트가 끝났음을 나타낸다.
  
  `ListFan<a>`의 사용자는 위의 두 경우를 구분하지 않아도 된다.

**팬 변환은 *온라인 알고리즘*이라는 점 역시 중요하다.** 즉, 입력의 각 부분이 주어질 때마다 대응되는 부분이 즉시 출력된다. Par의 동시성 실행 모형을 고려하면 변환으로 인해 동시성이나 시간차 정보를 전혀 잃지 않음을 알 수 있다.

**이제는 `MergeLists`를 쉽게 구현할 수 있다.**

```par
dec MergeLists : [<a> List<List<a>>] List<a>
def MergeLists = [<a> lists] poll(ListFan(lists)) {
  fan => fan.case {
    .end! => submit(),
    .spawn(l) r => submit(l, r),
    .item(x) fan => .item(x) submit(fan),
  }
  else => .end!,
}
```

> 💡 **이론이 궁금하다면?** 위에서 다룬 팬 패턴으로 [**_Client-server sessions in linear logic_**](https://dl.acm.org/doi/abs/10.1145/3473567) 논문을 커버하고 있다. 논문에서 제안한 쌍대거듭제곱은 다음과 같이 정의할 수 있다.
>
> ```par
> type Cobang = dual ListFan<dual a>
> type Coquest = List<a>
> ```
> 
> 위 연결사의 공리 규칙은 `List<List<a>>` 대신 `ListFan<a>`를 직접 전달받는 것을 제외하면 `MergeLists` 함수와 동일하게 구현되어 있다.
